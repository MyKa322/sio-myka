//! SIO's elevated helper process.
//!
//! Launched by the unelevated UI through a UAC prompt, connects back over a named pipe
//! and performs privileged operations on request. See [`sio_core::protocol`] for the
//! wire format and the rationale behind the nonce handshake.
//!
//! This binary is never intended to be run directly by a user. It refuses to do
//! anything without a valid pipe name and nonce supplied by its parent.

// No console window: this process is spawned by the UI and has no interactive output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SIO_LOG")
                .unwrap_or_else(|_| "sio_broker=info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|a| a == "--pipe") {
        eprintln!(
            "sio-broker is a helper process for SIO and is not meant to be run directly.\n\
             It requires --pipe and --nonce arguments supplied by the main application."
        );
        return ExitCode::FAILURE;
    }

    // Implemented in M2.
    tracing::error!("broker transport is not implemented yet");
    ExitCode::FAILURE
}
