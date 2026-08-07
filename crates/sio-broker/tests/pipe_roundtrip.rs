//! End-to-end test against the real broker binary over a real named pipe.
//!
//! The unit tests in `sio-winsys::broker` drive the protocol over an in-memory duplex,
//! which proves the logic but not the plumbing. This starts the actual `sio-broker.exe`,
//! connects to it through an actual named pipe, and performs a real registry operation.
//!
//! The one thing it deliberately does not exercise is `ShellExecuteExW`/`runas`: the
//! broker is spawned unelevated, because a UAC prompt cannot be answered by a test.
//! Everything downstream of elevation — pipe creation, the nonce handshake, frame
//! routing, request execution and the reply path — is covered here. Operations are
//! confined to HKCU so no administrator rights are needed.

#![cfg(windows)]

use sio_core::privileged::PrivilegedOps;
use sio_core::tweak::{Hive, PriorValue, RegistryEdit, RegistryValue};
use sio_winsys::broker::Session;
use std::time::Duration;
use tokio::net::windows::named_pipe::ServerOptions;

const BROKER_EXE: &str = env!("CARGO_BIN_EXE_sio-broker");
const TEST_KEY: &str = r"Software\SioTest\PipeRoundtrip";

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS RNG");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Start the broker unelevated and shake hands with it over a real pipe.
async fn connect() -> (Session, tokio::process::Child) {
    let pipe_name = format!(r"\\.\pipe\sio-broker-test-{}", random_hex(16));
    let nonce = random_hex(32);

    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe_name)
        .expect("could not create the test pipe");

    let child = tokio::process::Command::new(BROKER_EXE)
        .args(["--pipe", &pipe_name, "--nonce", &nonce])
        .kill_on_drop(true)
        .spawn()
        .expect("could not start the broker binary");

    tokio::time::timeout(Duration::from_secs(20), server.connect())
        .await
        .expect("the broker did not connect in time")
        .expect("the connection failed");

    let (reader, writer) = tokio::io::split(server);
    let session = Session::handshake(reader, writer, &nonce)
        .await
        .expect("the handshake should succeed against the real broker");

    (session, child)
}

#[tokio::test]
async fn the_real_broker_performs_a_registry_round_trip_over_a_pipe() {
    let (session, _child) = connect().await;

    let edit = RegistryEdit {
        hive: Hive::Hkcu,
        path: TEST_KEY.into(),
        name: "Value".into(),
        value: RegistryValue::Dword(1234),
    };

    // Write through the broker and get the prior state back across the wire.
    let prior = session
        .registry_set(&edit)
        .await
        .expect("the write should succeed");
    assert!(
        matches!(prior, PriorValue::Absent | PriorValue::KeyAbsent),
        "nothing was there to begin with, got {prior:?}"
    );

    // Confirm the elevated side really wrote it, reading directly rather than
    // trusting the broker's own reply.
    let observed = sio_winsys::registry::read_value(
        sio_winsys::registry::hkey_for(Hive::Hkcu),
        TEST_KEY,
        "Value",
    )
    .unwrap();
    assert_eq!(observed, Some(RegistryValue::Dword(1234)));

    // Revert, and confirm the value is gone rather than set to some default.
    session
        .registry_restore(Hive::Hkcu, TEST_KEY, "Value", &prior)
        .await
        .expect("the restore should succeed");

    let after = sio_winsys::registry::read_value(
        sio_winsys::registry::hkey_for(Hive::Hkcu),
        TEST_KEY,
        "Value",
    )
    .unwrap();
    assert_eq!(
        after, None,
        "reverting an absent prior must delete the value"
    );

    session.shutdown().await.ok();
}

#[tokio::test]
async fn progress_streams_from_the_real_broker_while_a_command_runs() {
    use sio_core::package::{PackageCmd, PackageOp, ProviderId};
    use sio_core::progress::{Progress, ProgressSink};

    let (session, _child) = connect().await;
    let (sink, mut rx) = ProgressSink::new();

    let cmd = PackageCmd {
        provider: ProviderId::Winget,
        op: PackageOp::Install,
        program: "cmd".into(),
        args: vec!["/C".into(), "echo broker-streamed-this".into()],
        elevated: false,
    };

    let code = session
        .run_package_cmd(&cmd, sink)
        .await
        .expect("the command should run");
    assert_eq!(code, 0);

    let mut lines = Vec::new();
    while let Ok(Progress::Log { line }) = rx.try_recv() {
        lines.push(line);
    }
    assert!(
        lines.iter().any(|l| l.contains("broker-streamed-this")),
        "child output must reach us across the pipe, got {lines:?}"
    );

    session.shutdown().await.ok();
}

#[tokio::test]
async fn the_real_broker_refuses_a_wrong_nonce() {
    // The squatting defence, against the actual binary rather than a stub.
    let pipe_name = format!(r"\\.\pipe\sio-broker-test-{}", random_hex(16));
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe_name)
        .unwrap();

    let mut child = tokio::process::Command::new(BROKER_EXE)
        .args(["--pipe", &pipe_name, "--nonce", &random_hex(32)])
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    tokio::time::timeout(Duration::from_secs(20), server.connect())
        .await
        .unwrap()
        .unwrap();
    let (reader, writer) = tokio::io::split(server);

    // We expect a *different* nonce than the one the broker was given.
    let err = Session::handshake(reader, writer, &random_hex(32))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("nonce"), "got {err}");

    // And the broker must exit rather than linger with elevated rights.
    let exited = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
    assert!(exited.is_ok(), "a rejected broker must not keep running");
}

#[tokio::test]
async fn the_broker_refuses_to_run_without_a_pipe_and_nonce() {
    // Someone double-clicking sio-broker.exe must get an explanation, not an idle
    // elevated agent waiting for instructions.
    let output = tokio::process::Command::new(BROKER_EXE)
        .output()
        .await
        .expect("the broker binary should be runnable");

    assert!(!output.status.success(), "it must refuse to start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not meant to be run directly"),
        "got {stderr:?}"
    );
}
