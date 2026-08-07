//! SIO's elevated helper process.
//!
//! Launched by the unelevated UI through a UAC prompt, connects back over a named pipe
//! and performs privileged operations on request. See [`sio_core::protocol`] for the
//! wire format and the reasoning behind the nonce handshake.
//!
//! This binary is not meant to be run directly and refuses to act without a pipe name
//! and nonce from its parent. Even given those, it does nothing until the parent has
//! accepted its handshake — so a user who double-clicks it gets an explanation, not an
//! elevated agent waiting for instructions.

// No console window: this process is spawned by the UI and has no interactive output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

/// Minimal flag parsing. A dependency would be more than this needs, and the argument
/// surface must stay deliberately tiny.
fn flag(args: &[String], name: &str) -> Option<String> {
    let index = args.iter().position(|a| a == name)?;
    args.get(index + 1).cloned()
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SIO_LOG")
                .unwrap_or_else(|_| "sio_broker=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(pipe), Some(nonce)) = (flag(&args, "--pipe"), flag(&args, "--nonce")) else {
        eprintln!(
            "sio-broker is a helper process for SIO and is not meant to be run directly.\n\
             It is started automatically when SIO needs administrator rights."
        );
        return ExitCode::FAILURE;
    };

    // A single-threaded runtime is plenty: this process waits on I/O and child
    // processes, and keeping it small keeps the elevated surface small.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            tracing::error!("could not start the runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(sio_winsys::broker::serve_pipe(&pipe, &nonce)) {
        Ok(()) => {
            tracing::info!("broker finished cleanly");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!("broker stopped: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_a_flag_value() {
        let args = args(&["--pipe", r"\\.\pipe\sio-broker-abc", "--nonce", "deadbeef"]);
        assert_eq!(
            flag(&args, "--pipe").as_deref(),
            Some(r"\\.\pipe\sio-broker-abc")
        );
        assert_eq!(flag(&args, "--nonce").as_deref(), Some("deadbeef"));
    }

    #[test]
    fn a_missing_flag_is_none() {
        assert_eq!(flag(&args(&["--pipe", "x"]), "--nonce"), None);
    }

    #[test]
    fn a_flag_without_a_value_is_none_rather_than_a_panic() {
        // Guards against an index-out-of-bounds on `sio-broker.exe --nonce`.
        assert_eq!(flag(&args(&["--pipe"]), "--pipe"), None);
    }

    #[test]
    fn no_arguments_yields_nothing() {
        assert_eq!(flag(&[], "--pipe"), None);
    }
}
