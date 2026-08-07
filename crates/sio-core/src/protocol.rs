//! Wire protocol between the unelevated UI and the elevated broker.
//!
//! # Why a named pipe
//!
//! A process elevated through `ShellExecuteExW`/`runas` cannot inherit stdio handles
//! from its unelevated parent, so the usual "spawn a child and talk over stdin/stdout"
//! approach is unavailable. A named pipe is the standard alternative.
//!
//! # Why a nonce
//!
//! Pipe names are guessable, and any local process can create a pipe. Without a shared
//! secret, a hostile process could squat the name we are about to use and receive
//! commands intended for the broker. The UI therefore generates a random pipe name
//! *and* a 256-bit nonce, passes both on the broker's command line, and drops any
//! connection that cannot present the nonce.
//!
//! Framing is newline-delimited JSON: trivially debuggable, and there is no
//! performance case for anything denser at this message volume.

use crate::package::PackageCmd;
use crate::privileged::RestorePointOutcome;
use crate::progress::Progress;
use crate::tweak::{AppxRef, Hive, PriorState, PriorValue, RegistryEdit, ServiceConfig};
use serde::{Deserialize, Serialize};

/// Incremented on any breaking change to the frames below. A version mismatch is a
/// hard failure: a stale broker left over from a previous install must not be trusted
/// to interpret our commands.
pub const PROTOCOL_VERSION: u32 = 1;

/// Nonce length in bytes. 256 bits — far beyond brute force for a value that lives for
/// the lifetime of one process.
pub const NONCE_BYTES: usize = 32;

/// A request from the UI to the broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BrokerOp {
    RegistrySet {
        edit: RegistryEdit,
    },
    RegistryRestore {
        hive: Hive,
        path: String,
        name: String,
        prior: PriorValue,
    },
    ServiceConfigure {
        config: ServiceConfig,
    },
    ServiceRestore {
        prior: PriorState,
    },
    AppxRemove {
        package: AppxRef,
    },
    CreateRestorePoint {
        description: String,
    },
    RunPackageCmd {
        command: PackageCmd,
    },
}

/// A successful operation's payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum OpOutput {
    /// Nothing to return; the operation either worked or errored.
    Unit,
    PriorValue(PriorValue),
    PriorState(PriorState),
    RestorePoint(RestorePointOutcome),
    ExitCode(i32),
}

/// An error carried across the pipe.
///
/// [`crate::error::Error`] is not serializable (it wraps `io::Error`), and shouldn't be —
/// the broker's internal error detail is not automatically safe or useful to the UI.
/// This is the deliberate, minimal projection of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireError {
    pub kind: WireErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireErrorKind {
    Registry,
    Service,
    Appx,
    RestorePoint,
    Process,
    /// The broker understood the frame but refuses to act on it.
    Refused,
    Internal,
}

/// Frames sent UI → broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerFrame {
    /// Handshake accepted; the broker may begin serving requests.
    Accept,
    /// Handshake rejected. The broker must exit without performing any work.
    Reject {
        reason: String,
    },
    Request {
        id: u64,
        op: BrokerOp,
    },
    /// Graceful shutdown request.
    Shutdown,
}

/// Frames sent broker → UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientFrame {
    /// First frame on the connection. Proves the sender was launched by us.
    Hello {
        protocol_version: u32,
        nonce: String,
    },
    Progress {
        request_id: u64,
        progress: Progress,
    },
    Response {
        request_id: u64,
        result: Result<OpOutput, WireError>,
    },
}

/// Compare two secrets without leaking their contents through timing.
///
/// Overkill for a local pipe, but it costs three lines and removes a whole class of
/// question from review.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Validate a broker's opening frame.
pub fn verify_hello(frame: &ClientFrame, expected_nonce: &str) -> std::result::Result<(), String> {
    let ClientFrame::Hello {
        protocol_version,
        nonce,
    } = frame
    else {
        return Err("first frame was not a hello".into());
    };
    if *protocol_version != PROTOCOL_VERSION {
        return Err(format!(
            "protocol version mismatch: broker speaks {protocol_version}, we speak {PROTOCOL_VERSION}"
        ));
    }
    if !constant_time_eq(nonce.as_bytes(), expected_nonce.as_bytes()) {
        return Err("nonce mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn hello(nonce: &str, version: u32) -> ClientFrame {
        ClientFrame::Hello {
            protocol_version: version,
            nonce: nonce.into(),
        }
    }

    #[test]
    fn correct_hello_is_accepted() {
        assert!(verify_hello(&hello(NONCE, PROTOCOL_VERSION), NONCE).is_ok());
    }

    #[test]
    fn wrong_nonce_is_rejected() {
        let attacker = hello("f".repeat(NONCE.len()).as_str(), PROTOCOL_VERSION);
        let err = verify_hello(&attacker, NONCE).unwrap_err();
        assert!(err.contains("nonce"), "got: {err}");
    }

    #[test]
    fn truncated_nonce_is_rejected() {
        let err = verify_hello(&hello("0123", PROTOCOL_VERSION), NONCE).unwrap_err();
        assert!(err.contains("nonce"), "got: {err}");
    }

    #[test]
    fn version_mismatch_is_rejected_before_any_work() {
        // A stale broker from a previous install must not be trusted.
        let err = verify_hello(&hello(NONCE, PROTOCOL_VERSION + 1), NONCE).unwrap_err();
        assert!(err.contains("protocol version"), "got: {err}");
    }

    #[test]
    fn a_non_hello_first_frame_is_rejected() {
        // Skipping the handshake must not be a way to start issuing commands.
        let sneaky = ClientFrame::Response {
            request_id: 1,
            result: Ok(OpOutput::Unit),
        };
        assert!(verify_hello(&sneaky, NONCE).is_err());
    }

    #[test]
    fn constant_time_eq_matches_normal_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn frames_are_newline_free_so_the_framing_holds() {
        // JSON escapes control characters, so a payload containing a newline cannot
        // desynchronise the line-delimited stream.
        let frame = ClientFrame::Progress {
            request_id: 1,
            progress: Progress::Log {
                line: "line one\nline two".into(),
            },
        };
        let encoded = serde_json::to_string(&frame).unwrap();
        assert!(
            !encoded.contains('\n'),
            "encoded frame must occupy exactly one line"
        );
        assert_eq!(
            serde_json::from_str::<ClientFrame>(&encoded).unwrap(),
            frame
        );
    }

    #[test]
    fn error_responses_round_trip() {
        let frame = ClientFrame::Response {
            request_id: 9,
            result: Err(WireError {
                kind: WireErrorKind::Registry,
                message: "access denied".into(),
            }),
        };
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(serde_json::from_str::<ClientFrame>(&json).unwrap(), frame);
    }

    #[test]
    fn broker_ops_round_trip() {
        let op = BrokerOp::RegistrySet {
            edit: RegistryEdit {
                hive: Hive::Hklm,
                path: "SOFTWARE\\Test".into(),
                name: "Value".into(),
                value: crate::tweak::RegistryValue::Dword(1),
            },
        };
        let frame = ServerFrame::Request { id: 1, op };
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(serde_json::from_str::<ServerFrame>(&json).unwrap(), frame);
    }
}
