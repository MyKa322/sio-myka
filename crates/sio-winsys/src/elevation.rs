//! Launching a process with administrator rights, and asking whether we already have
//! them.
//!
//! `ShellExecuteExW` with the `runas` verb is the only supported way to raise
//! privileges: `CreateProcess` cannot elevate. That choice has a consequence which
//! shapes the whole broker design — a process started this way **cannot inherit stdio
//! handles**, so the parent and child cannot talk over pipes on fds 0/1/2. Hence the
//! named pipe in [`sio_core::protocol`].

use sio_core::error::{Error, Result};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_CANCELLED, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// Whether the current process is running elevated.
///
/// Used to skip the broker entirely when the app was itself started as administrator —
/// prompting for rights we already hold would be nonsense.
pub fn is_elevated() -> Result<bool> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(|e| {
            Error::Windows {
                api: "OpenProcessToken".into(),
                reason: e.to_string(),
            }
        })?;
        let _token = OwnedHandle(token);

        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        GetTokenInformation(
            token,
            TokenElevation,
            Some(std::ptr::addr_of_mut!(elevation).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
        .map_err(|e| Error::Windows {
            api: "GetTokenInformation".into(),
            reason: e.to_string(),
        })?;

        Ok(elevation.TokenIsElevated != 0)
    }
}

/// An owned Win32 handle, closed on drop.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// A process we started with elevated rights.
#[derive(Debug)]
pub struct ElevatedProcess {
    handle: HANDLE,
}

// The handle is owned solely by this struct and only used through &self methods that
// take no interior locks, so moving it across threads is sound.
unsafe impl Send for ElevatedProcess {}
unsafe impl Sync for ElevatedProcess {}

impl ElevatedProcess {
    /// Whether the process is still running.
    ///
    /// Lets the client tell "the broker died" apart from "the broker is slow", which
    /// otherwise both present as a silent pipe.
    pub fn is_running(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle, 0) == WAIT_TIMEOUT }
    }

    /// Exit code, or `None` while still running.
    pub fn exit_code(&self) -> Option<u32> {
        if self.is_running() {
            return None;
        }
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.handle, &mut code).ok()? };
        Some(code)
    }

    /// Block until the process exits, up to `timeout_ms`. Returns whether it exited.
    pub fn wait(&self, timeout_ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.handle, timeout_ms) == WAIT_OBJECT_0 }
    }
}

impl Drop for ElevatedProcess {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

fn to_wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Quote one command-line argument per the Windows parsing rules.
///
/// Windows joins arguments into a single string and each program re-splits it, so an
/// unquoted path containing a space silently becomes two arguments. Our pipe names and
/// nonces are safe by construction, but the executable path is not.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"', '\n']) {
        return arg.to_string();
    }

    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => {
                backslashes += 1;
                out.push(ch);
            }
            '"' => {
                // Backslashes immediately before a quote must be doubled, and the quote
                // itself escaped.
                for _ in 0..=backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push('"');
            }
            _ => {
                backslashes = 0;
                out.push(ch);
            }
        }
    }
    // Trailing backslashes would otherwise escape the closing quote.
    for _ in 0..backslashes {
        out.push('\\');
    }
    out.push('"');
    out
}

/// Launch `exe` elevated, showing the UAC prompt.
///
/// Returns [`Error::ElevationDeclined`] when the user dismisses the prompt. That is a
/// normal outcome, not a failure, and callers are expected to treat it as a quiet
/// cancellation rather than an error dialog.
pub fn launch_elevated(exe: &Path, args: &[String]) -> Result<ElevatedProcess> {
    let exe_wide = to_wide(&exe.to_string_lossy());
    let verb = to_wide("runas");
    let joined = args
        .iter()
        .map(|a| quote_arg(a))
        .collect::<Vec<_>>()
        .join(" ");
    let params = to_wide(&joined);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        // NOCLOSEPROCESS gives us the process handle so we can detect a crash;
        // NOASYNC keeps the call valid without a message pump.
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(exe_wide.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };

    unsafe {
        ShellExecuteExW(&mut info).map_err(|e| {
            if e.code().0 as u32 & 0xFFFF == ERROR_CANCELLED.0 {
                Error::ElevationDeclined
            } else {
                Error::Windows {
                    api: "ShellExecuteExW".into(),
                    reason: e.to_string(),
                }
            }
        })?;
    }

    if info.hProcess.is_invalid() {
        return Err(Error::Windows {
            api: "ShellExecuteExW".into(),
            reason: "no process handle was returned".into(),
        });
    }

    Ok(ElevatedProcess {
        handle: info.hProcess,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_check_succeeds_and_is_consistent() {
        // We cannot assert which value is correct without knowing how the test runner
        // was started, but the call must succeed and be stable.
        let first = is_elevated().expect("querying our own token must not fail");
        assert_eq!(first, is_elevated().unwrap());
    }

    #[test]
    fn simple_arguments_are_left_alone() {
        assert_eq!(quote_arg("--pipe"), "--pipe");
        assert_eq!(quote_arg("abc123"), "abc123");
    }

    #[test]
    fn paths_with_spaces_are_quoted() {
        assert_eq!(
            quote_arg(r"C:\Program Files\SIO\sio-broker.exe"),
            "\"C:\\Program Files\\SIO\\sio-broker.exe\""
        );
    }

    #[test]
    fn empty_argument_becomes_an_explicit_empty_string() {
        // Otherwise it would vanish and shift every later argument by one position.
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn trailing_backslashes_do_not_escape_the_closing_quote() {
        // `"C:\dir\"` would be parsed as an unterminated string.
        assert_eq!(quote_arg(r"C:\dir with space\"), r#""C:\dir with space\\""#);
    }

    #[test]
    fn embedded_quotes_are_escaped() {
        assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
    }

    #[test]
    fn backslashes_before_a_quote_are_doubled() {
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
    }
}
