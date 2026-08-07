//! Newline-delimited JSON framing, shared by both ends of the broker pipe.
//!
//! Generic over any async stream so the transport (a named pipe in production, an
//! in-memory duplex in tests) is interchangeable. That is what lets the handshake and
//! request/response logic be tested without ever showing a UAC prompt.
//!
//! Framing is safe because `serde_json` escapes control characters: no encoded frame
//! can contain a literal newline, so a payload can never desynchronise the stream.
//! [`crate::protocol`] has a test pinning that property.

use crate::error::{Error, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};

/// A frame larger than this is treated as hostile rather than merely surprising.
///
/// Without a cap, a peer that never sends a newline makes us buffer until the process
/// dies. Real frames are a few hundred bytes; the ceiling is generous.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Reads frames of type `T` from an async stream.
#[derive(Debug)]
pub struct FrameReader<R> {
    lines: Lines<BufReader<R>>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            lines: BufReader::new(reader).lines(),
        }
    }

    /// Read the next frame. `Ok(None)` means the peer closed the connection cleanly.
    pub async fn next<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
        loop {
            let Some(line) = self.lines.next_line().await? else {
                return Ok(None);
            };

            // Tolerate blank lines so a stray newline is not a protocol violation.
            if line.trim().is_empty() {
                continue;
            }
            if line.len() > MAX_FRAME_BYTES {
                return Err(Error::Broker {
                    reason: format!("frame of {} bytes exceeds the limit", line.len()),
                });
            }

            return serde_json::from_str(&line)
                .map(Some)
                .map_err(|e| Error::Broker {
                    reason: format!("malformed frame: {e}"),
                });
        }
    }
}

/// Writes frames to an async stream.
#[derive(Debug)]
pub struct FrameWriter<W> {
    inner: W,
}

impl<W: AsyncWrite + Unpin> FrameWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { inner: writer }
    }

    /// Serialize and send one frame, flushing so the peer sees it immediately.
    ///
    /// Flushing every frame matters for progress updates: a buffered stream would make
    /// a long install look frozen.
    pub async fn send<T: Serialize>(&mut self, frame: &T) -> Result<()> {
        let mut bytes = serde_json::to_vec(frame)?;
        bytes.push(b'\n');
        self.inner.write_all(&bytes).await?;
        self.inner.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Msg {
        id: u32,
        text: String,
    }

    #[tokio::test]
    async fn frames_round_trip_in_order() {
        let (client, server) = tokio::io::duplex(4096);
        let mut writer = FrameWriter::new(client);
        let mut reader = FrameReader::new(server);

        writer
            .send(&Msg {
                id: 1,
                text: "one".into(),
            })
            .await
            .unwrap();
        writer
            .send(&Msg {
                id: 2,
                text: "two".into(),
            })
            .await
            .unwrap();

        assert_eq!(reader.next::<Msg>().await.unwrap().unwrap().id, 1);
        assert_eq!(reader.next::<Msg>().await.unwrap().unwrap().id, 2);
    }

    #[tokio::test]
    async fn payload_newlines_do_not_split_a_frame() {
        let (client, server) = tokio::io::duplex(4096);
        let mut writer = FrameWriter::new(client);
        let mut reader = FrameReader::new(server);

        let tricky = Msg {
            id: 7,
            text: "line one\nline two\r\nthree".into(),
        };
        writer.send(&tricky).await.unwrap();

        assert_eq!(reader.next::<Msg>().await.unwrap().unwrap(), tricky);
    }

    #[tokio::test]
    async fn closed_stream_yields_none_rather_than_an_error() {
        let (client, server) = tokio::io::duplex(64);
        drop(client);
        let mut reader = FrameReader::new(server);
        assert!(reader.next::<Msg>().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn malformed_json_is_reported_as_a_broker_error() {
        let (mut client, server) = tokio::io::duplex(4096);
        client.write_all(b"{not json at all}\n").await.unwrap();
        let mut reader = FrameReader::new(server);

        let err = reader.next::<Msg>().await.unwrap_err();
        assert!(matches!(err, Error::Broker { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn blank_lines_are_skipped() {
        let (mut client, server) = tokio::io::duplex(4096);
        client
            .write_all(b"\n\n{\"id\":3,\"text\":\"ok\"}\n")
            .await
            .unwrap();
        let mut reader = FrameReader::new(server);

        assert_eq!(reader.next::<Msg>().await.unwrap().unwrap().id, 3);
    }

    #[tokio::test]
    async fn oversized_frames_are_rejected() {
        let (mut client, server) = tokio::io::duplex(MAX_FRAME_BYTES * 2);
        let giant = format!("{}\n", "x".repeat(MAX_FRAME_BYTES + 10));
        client.write_all(giant.as_bytes()).await.unwrap();
        let mut reader = FrameReader::new(server);

        let err = reader.next::<Msg>().await.unwrap_err();
        assert!(err.to_string().contains("exceeds the limit"), "got {err}");
    }
}
