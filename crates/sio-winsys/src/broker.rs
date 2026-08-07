//! The two halves of the elevated-broker conversation.
//!
//! [`Session`] is the unelevated client; [`serve`] is the elevated server. Both are
//! generic over the stream, so the whole protocol — handshake, request routing,
//! progress streaming, shutdown — is exercised in tests over an in-memory duplex,
//! without a named pipe and without a UAC prompt. Only [`Session::launch`] involves
//! either.

use crate::elevation::{self, ElevatedProcess};
use crate::ops::InProcessOps;
use async_trait::async_trait;
use sio_core::codec::{FrameReader, FrameWriter};
use sio_core::error::{Error, Result};
use sio_core::package::PackageCmd;
use sio_core::privileged::{PrivilegedOps, RestorePointOutcome};
use sio_core::progress::ProgressSink;
use sio_core::protocol::{
    verify_hello, BrokerOp, ClientFrame, OpOutput, ServerFrame, WireError, WireErrorKind,
    NONCE_BYTES, PROTOCOL_VERSION,
};
use sio_core::tweak::{AppxRef, Hive, PriorState, PriorValue, RegistryEdit, ServiceConfig};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{oneshot, Mutex};

/// How long to wait for the elevated process to connect back.
///
/// Generous because the clock starts when the UAC dialog appears, and a user may take a
/// while to notice it.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

type BoxedWriter = Box<dyn AsyncWrite + Send + Unpin>;
type Reply = std::result::Result<OpOutput, WireError>;

/// Generate a random hex nonce using the OS CSPRNG.
fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the OS random source must be available");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// A live conversation with the broker.
pub struct Session {
    writer: Arc<Mutex<FrameWriter<BoxedWriter>>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>>,
    sinks: Arc<Mutex<HashMap<u64, ProgressSink>>>,
    next_id: AtomicU64,
    /// Kept alive so the handle stays open and liveness can be checked. `None` when the
    /// session is driven over a test stream.
    process: Option<ElevatedProcess>,
}

// Derived Debug is impossible over a boxed writer, and the writer's contents are not
// something a log should print anyway.
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("elevated", &self.process.is_some())
            .field("alive", &self.is_alive())
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Perform the client side of the handshake over an already-connected stream.
    pub async fn handshake<R, W>(reader: R, writer: W, expected_nonce: &str) -> Result<Self>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let mut frames = FrameReader::new(reader);
        let mut writer = FrameWriter::new(Box::new(writer) as BoxedWriter);

        let hello: ClientFrame = frames.next().await?.ok_or_else(|| Error::Broker {
            reason: "the helper closed before saying hello".into(),
        })?;

        if let Err(reason) = verify_hello(&hello, expected_nonce) {
            // Tell the peer why, then refuse. A process that cannot prove it is ours
            // must never reach the request loop.
            let _ = writer
                .send(&ServerFrame::Reject {
                    reason: reason.clone(),
                })
                .await;
            return Err(Error::Broker { reason });
        }
        writer.send(&ServerFrame::Accept).await?;

        let session = Self {
            writer: Arc::new(Mutex::new(writer)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            sinks: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            process: None,
        };

        session.spawn_reader(frames);
        Ok(session)
    }

    /// Start the elevated helper and shake hands with it.
    ///
    /// Shows a UAC prompt. [`Error::ElevationDeclined`] means the user dismissed it,
    /// which callers should treat as a quiet cancellation.
    pub async fn launch(broker_exe: &Path) -> Result<Self> {
        use tokio::net::windows::named_pipe::ServerOptions;

        // A random name plus `first_pipe_instance` means a hostile process can neither
        // guess the name nor hijack it by creating it first — the create would fail.
        let pipe_name = format!(r"\\.\pipe\sio-broker-{}", random_hex(16));
        let nonce = random_hex(NONCE_BYTES);

        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&pipe_name)
            .map_err(|e| Error::Broker {
                reason: format!("could not create the pipe: {e}"),
            })?;

        let process = elevation::launch_elevated(
            broker_exe,
            &[
                "--pipe".into(),
                pipe_name.clone(),
                "--nonce".into(),
                nonce.clone(),
            ],
        )?;

        // Race the connection against the helper dying. Without this a broker that
        // crashes on startup would leave us waiting the full timeout for a peer that
        // is never coming.
        tokio::time::timeout(CONNECT_TIMEOUT, async {
            let mut connect = std::pin::pin!(server.connect());
            loop {
                tokio::select! {
                    result = &mut connect => return result.map_err(|e| Error::Broker {
                        reason: format!("the helper failed to connect: {e}"),
                    }),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {
                        if !process.is_running() {
                            return Err(Error::Broker {
                                reason: format!(
                                    "the helper exited before connecting (code {:?})",
                                    process.exit_code()
                                ),
                            });
                        }
                    }
                }
            }
        })
        .await
        .map_err(|_| Error::Broker {
            reason: "the helper did not connect in time".into(),
        })??;

        let (reader, writer) = tokio::io::split(server);
        let mut session = Self::handshake(reader, writer, &nonce).await?;
        session.process = Some(process);
        Ok(session)
    }

    /// Whether the helper is still alive. Always true for a test session.
    pub fn is_alive(&self) -> bool {
        self.process
            .as_ref()
            .is_none_or(ElevatedProcess::is_running)
    }

    fn spawn_reader<R>(&self, mut frames: FrameReader<R>)
    where
        R: AsyncRead + Send + Unpin + 'static,
    {
        let pending = Arc::clone(&self.pending);
        let sinks = Arc::clone(&self.sinks);

        tokio::spawn(async move {
            loop {
                match frames.next::<ClientFrame>().await {
                    Ok(Some(ClientFrame::Progress {
                        request_id,
                        progress,
                    })) => {
                        let sink = sinks.lock().await.get(&request_id).cloned();
                        if let Some(sink) = sink {
                            sink.send(progress);
                        }
                    }
                    Ok(Some(ClientFrame::Response { request_id, result })) => {
                        sinks.lock().await.remove(&request_id);
                        if let Some(tx) = pending.lock().await.remove(&request_id) {
                            let _ = tx.send(result);
                        }
                    }
                    // A second hello is a protocol violation; ignore rather than trust.
                    Ok(Some(ClientFrame::Hello { .. })) => continue,
                    Ok(None) | Err(_) => break,
                }
            }

            // The pipe closed. Fail every outstanding request rather than leaving
            // callers awaiting a reply that can never arrive.
            let mut pending = pending.lock().await;
            for (_, tx) in pending.drain() {
                let _ = tx.send(Err(WireError {
                    kind: WireErrorKind::Internal,
                    message: "the elevated helper disconnected".into(),
                }));
            }
        });
    }

    /// Send an operation and await its result.
    pub async fn request(&self, op: BrokerOp, progress: ProgressSink) -> Result<OpOutput> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();

        self.pending.lock().await.insert(id, tx);
        self.sinks.lock().await.insert(id, progress);

        if let Err(e) = self
            .writer
            .lock()
            .await
            .send(&ServerFrame::Request { id, op })
            .await
        {
            self.pending.lock().await.remove(&id);
            self.sinks.lock().await.remove(&id);
            return Err(e);
        }

        match rx.await {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(wire)) => Err(Error::Broker {
                reason: format!("{:?}: {}", wire.kind, wire.message),
            }),
            Err(_) => Err(Error::Broker {
                reason: "the helper stopped responding".into(),
            }),
        }
    }

    /// Ask the helper to exit.
    pub async fn shutdown(&self) -> Result<()> {
        self.writer.lock().await.send(&ServerFrame::Shutdown).await
    }
}

/// Unwrap an expected payload shape, so a protocol mismatch is a clear error rather
/// than a silent wrong value.
fn expect_unit(output: OpOutput) -> Result<()> {
    match output {
        OpOutput::Unit => Ok(()),
        other => Err(Error::Broker {
            reason: format!("expected no payload, got {other:?}"),
        }),
    }
}

#[async_trait]
impl PrivilegedOps for Session {
    async fn registry_set(&self, edit: &RegistryEdit) -> Result<PriorValue> {
        match self
            .request(
                BrokerOp::RegistrySet { edit: edit.clone() },
                ProgressSink::null(),
            )
            .await?
        {
            OpOutput::PriorValue(v) => Ok(v),
            other => Err(Error::Broker {
                reason: format!("expected a prior value, got {other:?}"),
            }),
        }
    }

    async fn registry_restore(
        &self,
        hive: Hive,
        path: &str,
        name: &str,
        prior: &PriorValue,
    ) -> Result<()> {
        expect_unit(
            self.request(
                BrokerOp::RegistryRestore {
                    hive,
                    path: path.into(),
                    name: name.into(),
                    prior: prior.clone(),
                },
                ProgressSink::null(),
            )
            .await?,
        )
    }

    async fn service_configure(&self, cfg: &ServiceConfig) -> Result<PriorState> {
        match self
            .request(
                BrokerOp::ServiceConfigure {
                    config: cfg.clone(),
                },
                ProgressSink::null(),
            )
            .await?
        {
            OpOutput::PriorState(s) => Ok(s),
            other => Err(Error::Broker {
                reason: format!("expected a prior state, got {other:?}"),
            }),
        }
    }

    async fn service_restore(&self, prior: &PriorState) -> Result<()> {
        expect_unit(
            self.request(
                BrokerOp::ServiceRestore {
                    prior: prior.clone(),
                },
                ProgressSink::null(),
            )
            .await?,
        )
    }

    async fn appx_remove(&self, pkg: &AppxRef) -> Result<()> {
        expect_unit(
            self.request(
                BrokerOp::AppxRemove {
                    package: pkg.clone(),
                },
                ProgressSink::null(),
            )
            .await?,
        )
    }

    async fn create_restore_point(&self, description: &str) -> Result<RestorePointOutcome> {
        match self
            .request(
                BrokerOp::CreateRestorePoint {
                    description: description.into(),
                },
                ProgressSink::null(),
            )
            .await?
        {
            OpOutput::RestorePoint(outcome) => Ok(outcome),
            other => Err(Error::Broker {
                reason: format!("expected an outcome, got {other:?}"),
            }),
        }
    }

    async fn run_package_cmd(&self, cmd: &PackageCmd, progress: ProgressSink) -> Result<i32> {
        match self
            .request(
                BrokerOp::RunPackageCmd {
                    command: cmd.clone(),
                },
                progress,
            )
            .await?
        {
            OpOutput::ExitCode(code) => Ok(code),
            other => Err(Error::Broker {
                reason: format!("expected an exit code, got {other:?}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

fn to_wire(err: &Error, kind: WireErrorKind) -> WireError {
    WireError {
        kind,
        message: err.to_string(),
    }
}

/// Run the elevated side of the conversation until the peer disconnects or asks us to
/// stop.
///
/// Refuses to do any work until the handshake is accepted.
pub async fn serve<R, W>(
    reader: R,
    writer: W,
    nonce: &str,
    ops: Arc<dyn PrivilegedOps>,
) -> Result<()>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let mut frames = FrameReader::new(reader);
    let writer = Arc::new(Mutex::new(
        FrameWriter::new(Box::new(writer) as BoxedWriter),
    ));

    writer
        .lock()
        .await
        .send(&ClientFrame::Hello {
            protocol_version: PROTOCOL_VERSION,
            nonce: nonce.to_string(),
        })
        .await?;

    match frames.next::<ServerFrame>().await? {
        Some(ServerFrame::Accept) => {}
        Some(ServerFrame::Reject { reason }) => {
            return Err(Error::Broker {
                reason: format!("handshake rejected: {reason}"),
            })
        }
        _ => {
            return Err(Error::Broker {
                reason: "expected accept or reject".into(),
            })
        }
    }

    while let Some(frame) = frames.next::<ServerFrame>().await? {
        match frame {
            ServerFrame::Request { id, op } => {
                let writer = Arc::clone(&writer);
                let ops = Arc::clone(&ops);
                // Each request runs concurrently so a long install does not block a
                // quick registry read queued behind it.
                tokio::spawn(async move {
                    let result = execute(id, op, &ops, &writer).await;
                    let mut guard = writer.lock().await;

                    if let Err(e) = guard
                        .send(&ClientFrame::Response {
                            request_id: id,
                            result,
                        })
                        .await
                    {
                        // A reply that never goes out leaves the caller awaiting a
                        // oneshot that will never resolve — a hang, not an error. So
                        // never let a send failure pass silently: fall back to a frame
                        // that is guaranteed to encode.
                        tracing::error!("could not send the reply to request {id}: {e}");
                        let _ = guard
                            .send(&ClientFrame::Response {
                                request_id: id,
                                result: Err(WireError {
                                    kind: WireErrorKind::Internal,
                                    message: format!("the reply could not be encoded: {e}"),
                                }),
                            })
                            .await;
                    }
                });
            }
            ServerFrame::Shutdown => break,
            // The client never re-handshakes; ignore rather than act.
            ServerFrame::Accept | ServerFrame::Reject { .. } => continue,
        }
    }
    Ok(())
}

async fn execute(
    id: u64,
    op: BrokerOp,
    ops: &Arc<dyn PrivilegedOps>,
    writer: &Arc<Mutex<FrameWriter<BoxedWriter>>>,
) -> Reply {
    match op {
        BrokerOp::RegistrySet { edit } => ops
            .registry_set(&edit)
            .await
            .map(OpOutput::PriorValue)
            .map_err(|e| to_wire(&e, WireErrorKind::Registry)),

        BrokerOp::RegistryRestore {
            hive,
            path,
            name,
            prior,
        } => ops
            .registry_restore(hive, &path, &name, &prior)
            .await
            .map(|_| OpOutput::Unit)
            .map_err(|e| to_wire(&e, WireErrorKind::Registry)),

        BrokerOp::ServiceConfigure { config } => ops
            .service_configure(&config)
            .await
            .map(OpOutput::PriorState)
            .map_err(|e| to_wire(&e, WireErrorKind::Service)),

        BrokerOp::ServiceRestore { prior } => ops
            .service_restore(&prior)
            .await
            .map(|_| OpOutput::Unit)
            .map_err(|e| to_wire(&e, WireErrorKind::Service)),

        BrokerOp::AppxRemove { package } => ops
            .appx_remove(&package)
            .await
            .map(|_| OpOutput::Unit)
            .map_err(|e| to_wire(&e, WireErrorKind::Appx)),

        BrokerOp::CreateRestorePoint { description } => ops
            .create_restore_point(&description)
            .await
            .map(OpOutput::RestorePoint)
            .map_err(|e| to_wire(&e, WireErrorKind::RestorePoint)),

        BrokerOp::RunPackageCmd { command } => {
            // Forward this operation's progress back over the pipe as it happens.
            let (sink, mut rx) = ProgressSink::new();
            let forward_writer = Arc::clone(writer);
            let forwarder = tokio::spawn(async move {
                while let Some(progress) = rx.recv().await {
                    let _ = forward_writer
                        .lock()
                        .await
                        .send(&ClientFrame::Progress {
                            request_id: id,
                            progress,
                        })
                        .await;
                }
            });

            let result = ops.run_package_cmd(&command, sink).await;
            // Dropping our sink ends the forwarder once it has drained.
            let _ = forwarder.await;

            result
                .map(OpOutput::ExitCode)
                .map_err(|e| to_wire(&e, WireErrorKind::Process))
        }
    }
}

/// Convenience entry point for the broker binary.
pub async fn serve_pipe(pipe_name: &str, nonce: &str) -> Result<()> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let client = ClientOptions::new()
        .open(pipe_name)
        .map_err(|e| Error::Broker {
            reason: format!("could not open `{pipe_name}`: {e}"),
        })?;

    let (reader, writer) = tokio::io::split(client);
    serve(reader, writer, nonce, Arc::new(InProcessOps::new())).await
}

/// Where the elevated helper lives: next to the main executable.
pub fn broker_path() -> Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or_else(|| Error::Broker {
        reason: "could not determine the application directory".into(),
    })?;
    Ok(dir.join("sio-broker.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sio_core::progress::Progress;
    use sio_core::tweak::RegistryValue;

    const NONCE: &str = "a-test-nonce-that-both-sides-agree-on";

    /// Wire a client session to a server over two in-memory duplex streams.
    async fn connected(ops: Arc<dyn PrivilegedOps>) -> Session {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_r, client_w) = tokio::io::split(client_io);
        let (server_r, server_w) = tokio::io::split(server_io);

        tokio::spawn(async move {
            let _ = serve(server_r, server_w, NONCE, ops).await;
        });

        Session::handshake(client_r, client_w, NONCE)
            .await
            .expect("handshake should succeed")
    }

    #[derive(Debug, Default)]
    struct FakeOps;

    #[async_trait]
    impl PrivilegedOps for FakeOps {
        async fn registry_set(&self, _edit: &RegistryEdit) -> Result<PriorValue> {
            Ok(PriorValue::Present(RegistryValue::Dword(41)))
        }
        async fn registry_restore(
            &self,
            _h: Hive,
            _p: &str,
            _n: &str,
            _prior: &PriorValue,
        ) -> Result<()> {
            Ok(())
        }
        async fn service_configure(&self, cfg: &ServiceConfig) -> Result<PriorState> {
            Ok(PriorState {
                name: cfg.name.clone(),
                start_type: sio_core::tweak::ServiceStartType::Automatic,
                was_running: true,
            })
        }
        async fn service_restore(&self, _prior: &PriorState) -> Result<()> {
            Ok(())
        }
        async fn appx_remove(&self, _pkg: &AppxRef) -> Result<()> {
            Err(Error::Other("nope".into()))
        }
        async fn create_restore_point(&self, _d: &str) -> Result<RestorePointOutcome> {
            Ok(RestorePointOutcome::SkippedThrottled)
        }
        async fn run_package_cmd(&self, _c: &PackageCmd, progress: ProgressSink) -> Result<i32> {
            progress.log("downloading");
            progress.log("installing");
            Ok(0)
        }
    }

    fn sample_edit() -> RegistryEdit {
        RegistryEdit {
            hive: Hive::Hkcu,
            path: r"Software\SioTest".into(),
            name: "X".into(),
            value: RegistryValue::Dword(1),
        }
    }

    #[tokio::test]
    async fn a_request_gets_its_result_back() {
        let session = connected(Arc::new(FakeOps)).await;
        let prior = session.registry_set(&sample_edit()).await.unwrap();
        assert_eq!(prior, PriorValue::Present(RegistryValue::Dword(41)));
    }

    #[tokio::test]
    async fn errors_cross_the_wire_with_their_message_intact() {
        let session = connected(Arc::new(FakeOps)).await;
        let err = session
            .appx_remove(&AppxRef {
                package_family_name: "X_y".into(),
                deprovision: false,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nope"), "got {err}");
    }

    #[tokio::test]
    async fn progress_is_streamed_before_the_result() {
        let session = connected(Arc::new(FakeOps)).await;
        let (sink, mut rx) = ProgressSink::new();

        let cmd = PackageCmd {
            provider: sio_core::package::ProviderId::Winget,
            op: sio_core::package::PackageOp::Install,
            program: "irrelevant".into(),
            args: vec![],
            elevated: true,
        };
        let code = session.run_package_cmd(&cmd, sink).await.unwrap();
        assert_eq!(code, 0);

        let mut lines = Vec::new();
        while let Ok(Progress::Log { line }) = rx.try_recv() {
            lines.push(line);
        }
        assert_eq!(
            lines,
            vec!["downloading", "installing"],
            "progress must arrive in order"
        );
    }

    #[tokio::test]
    async fn concurrent_requests_are_matched_to_the_right_caller() {
        // Responses may arrive out of order; ids are what keep them straight.
        let session = Arc::new(connected(Arc::new(FakeOps)).await);

        let mut handles = Vec::new();
        for _ in 0..8 {
            let session = Arc::clone(&session);
            handles.push(tokio::spawn(async move {
                session.registry_set(&sample_edit()).await
            }));
        }
        for handle in handles {
            assert_eq!(
                handle.await.unwrap().unwrap(),
                PriorValue::Present(RegistryValue::Dword(41))
            );
        }
    }

    #[tokio::test]
    async fn a_wrong_nonce_is_rejected_and_no_work_is_done() {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_r, client_w) = tokio::io::split(client_io);
        let (server_r, server_w) = tokio::io::split(server_io);

        let served = tokio::spawn(async move {
            serve(server_r, server_w, "the-attackers-nonce", Arc::new(FakeOps)).await
        });

        let err = Session::handshake(client_r, client_w, NONCE)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nonce"), "got {err}");

        // The server must learn it was rejected and stop, rather than sit waiting.
        let server_result = served.await.unwrap();
        assert!(
            server_result.is_err(),
            "the server must abort after a rejected handshake"
        );
    }

    #[tokio::test]
    async fn pending_requests_fail_when_the_helper_disconnects() {
        // Rather than hanging forever, which is what a naive implementation does.
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_r, client_w) = tokio::io::split(client_io);
        let (server_r, server_w) = tokio::io::split(server_io);

        tokio::spawn(async move {
            let mut frames = FrameReader::new(server_r);
            let mut writer = FrameWriter::new(Box::new(server_w) as BoxedWriter);
            writer
                .send(&ClientFrame::Hello {
                    protocol_version: PROTOCOL_VERSION,
                    nonce: NONCE.into(),
                })
                .await
                .unwrap();
            let _: Option<ServerFrame> = frames.next().await.unwrap();
            // Read the request, then vanish without replying.
            let _: Option<ServerFrame> = frames.next().await.unwrap();
        });

        let session = Session::handshake(client_r, client_w, NONCE).await.unwrap();
        let err = session.registry_set(&sample_edit()).await.unwrap_err();
        assert!(
            err.to_string().contains("disconnected") || err.to_string().contains("responding"),
            "got {err}"
        );
    }

    #[tokio::test]
    async fn shutdown_ends_the_server_loop() {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_r, client_w) = tokio::io::split(client_io);
        let (server_r, server_w) = tokio::io::split(server_io);

        let served =
            tokio::spawn(async move { serve(server_r, server_w, NONCE, Arc::new(FakeOps)).await });

        let session = Session::handshake(client_r, client_w, NONCE).await.unwrap();
        session.shutdown().await.unwrap();

        assert!(
            served.await.unwrap().is_ok(),
            "a clean shutdown is not an error"
        );
    }

    #[test]
    fn nonces_are_unique_and_the_right_length() {
        let a = random_hex(NONCE_BYTES);
        let b = random_hex(NONCE_BYTES);
        assert_eq!(
            a.len(),
            NONCE_BYTES * 2,
            "hex encoding doubles the byte count"
        );
        assert_ne!(a, b, "nonces must not repeat");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn the_broker_is_expected_beside_the_main_executable() {
        let path = broker_path().unwrap();
        assert_eq!(path.file_name().unwrap(), "sio-broker.exe");
        assert_eq!(path.parent(), std::env::current_exe().unwrap().parent());
    }
}
