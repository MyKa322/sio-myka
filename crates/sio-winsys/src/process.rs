//! Running child processes and streaming their output.
//!
//! Used for package managers, Appx removal and restore points. Two rules apply
//! throughout:
//!
//! 1. **Decisions come from exit codes, never from output.** Package-manager text is
//!    localized — on a Russian Windows, winget answers in Russian — and reflows between
//!    versions. Output is for humans reading the log pane.
//! 2. **No console window.** Without `CREATE_NO_WINDOW` every child flashes a black
//!    box over the app.

use sio_core::error::{Error, Result};
use sio_core::progress::ProgressSink;
use tokio::io::{AsyncBufReadExt, BufReader};
// NB: `creation_flags` is an inherent method on tokio's Command under cfg(windows),
// so no extension trait import is needed here.
use tokio::process::Command;

/// `CREATE_NO_WINDOW`.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Run a command to completion, streaming stdout and stderr to `progress`.
///
/// Returns the raw exit code. Interpreting it is the caller's job, because only the
/// caller knows what its tool's codes mean.
pub async fn run_streaming(program: &str, args: &[String], progress: &ProgressSink) -> Result<i32> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::Other(format!("could not start `{program}`: {e}")))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Both streams are drained concurrently. Reading them in sequence would deadlock
    // as soon as a child filled the pipe we were not reading.
    let pump_out = pump(stdout, progress.clone());
    let pump_err = pump(stderr, progress.clone());
    tokio::join!(pump_out, pump_err);

    let status = child
        .wait()
        .await
        .map_err(|e| Error::Other(format!("`{program}` did not exit cleanly: {e}")))?;

    // A process killed by a signal has no code; report a sentinel rather than
    // pretending it succeeded.
    Ok(status.code().unwrap_or(-1))
}

async fn pump<R>(stream: Option<R>, progress: ProgressSink)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let Some(stream) = stream else { return };
    let mut lines = BufReader::new(stream).split(b'\n');

    // Split on bytes and decode lossily: Chocolatey writes in the console code page,
    // which is not UTF-8 on a Russian install. Since this text is only ever displayed,
    // a replacement character beats dropping the line.
    while let Ok(Some(chunk)) = lines.next_segment().await {
        let text = String::from_utf8_lossy(&chunk);
        let line = text.trim_end_matches(['\r', '\n']).trim_end();
        if !line.is_empty() {
            progress.log(line);
        }
    }
}

/// Run a command and capture stdout, without streaming.
///
/// For short, machine-readable output such as `winget export` or a CIM query.
pub async fn run_captured(program: &str, args: &[String]) -> Result<(i32, String)> {
    let output = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .map_err(|e| Error::Other(format!("could not start `{program}`: {e}")))?;

    Ok((
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sio_core::progress::{Progress, ProgressSink};

    #[tokio::test]
    async fn captures_stdout_and_exit_code() {
        let (code, out) = run_captured("cmd", &["/C".into(), "echo hello-from-test".into()])
            .await
            .unwrap();

        assert_eq!(code, 0);
        assert!(out.contains("hello-from-test"), "got {out:?}");
    }

    #[tokio::test]
    async fn non_zero_exit_codes_are_returned_not_turned_into_errors() {
        // Package managers signal "already installed" through non-zero codes, so a
        // non-zero exit must reach the caller intact rather than becoming an Err.
        let (code, _) = run_captured("cmd", &["/C".into(), "exit 3".into()])
            .await
            .unwrap();
        assert_eq!(code, 3);
    }

    #[tokio::test]
    async fn streams_output_lines_to_the_progress_sink() {
        let (sink, mut rx) = ProgressSink::new();
        let code = run_streaming(
            "cmd",
            &["/C".into(), "echo first & echo second".into()],
            &sink,
        )
        .await
        .unwrap();
        drop(sink);

        assert_eq!(code, 0);
        let mut logged = Vec::new();
        while let Ok(progress) = rx.try_recv() {
            if let Progress::Log { line } = progress {
                logged.push(line);
            }
        }
        assert!(logged.iter().any(|l| l.contains("first")), "got {logged:?}");
        assert!(
            logged.iter().any(|l| l.contains("second")),
            "got {logged:?}"
        );
    }

    #[tokio::test]
    async fn stderr_is_streamed_too() {
        let (sink, mut rx) = ProgressSink::new();
        run_streaming("cmd", &["/C".into(), "echo oops 1>&2".into()], &sink)
            .await
            .unwrap();
        drop(sink);

        let mut logged = Vec::new();
        while let Ok(progress) = rx.try_recv() {
            if let Progress::Log { line } = progress {
                logged.push(line);
            }
        }
        assert!(
            logged.iter().any(|l| l.contains("oops")),
            "stderr must reach the log pane"
        );
    }

    #[tokio::test]
    async fn a_missing_program_is_an_error_not_a_silent_exit_code() {
        let result = run_captured("sio-definitely-not-a-real-program", &[]).await;
        assert!(
            result.is_err(),
            "a missing executable must not look like a failed install"
        );
    }
}
