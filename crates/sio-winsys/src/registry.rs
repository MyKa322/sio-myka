//! Registry access.
//!
//! Thin, safe wrappers over the Win32 registry APIs. Read support is used by the
//! dashboard; read-then-write is the basis of the reversible tweak engine.

use sio_core::error::{Error, Result};
use sio_core::tweak::Hive;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY,
};

pub fn hkey_for(hive: Hive) -> HKEY {
    match hive {
        Hive::Hklm => HKEY_LOCAL_MACHINE,
        Hive::Hkcu => HKEY_CURRENT_USER,
        Hive::Hkcr => HKEY_CLASSES_ROOT,
        Hive::Hku => HKEY_USERS,
    }
}

fn registry_error(path: &str, code: WIN32_ERROR) -> Error {
    Error::Registry {
        path: path.to_string(),
        reason: format!("win32 error {}", code.0),
    }
}

/// Read a `REG_SZ` value.
///
/// Returns `Ok(None)` when the key or value simply does not exist — an expected
/// condition on Windows, not an error. Distinguishing "absent" from "failed to read"
/// matters: the tweak engine treats absent as a valid prior state to revert to.
pub fn read_string(root: HKEY, subkey: &str, value: &str) -> Result<Option<String>> {
    let subkey_w = HSTRING::from(subkey);
    let value_w = HSTRING::from(value);

    // First call sizes the buffer.
    let mut size: u32 = 0;
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey_w.as_ptr()),
            PCWSTR(value_w.as_ptr()),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            None,
            None,
            Some(&mut size),
        )
    };
    match status {
        ERROR_SUCCESS => {}
        ERROR_FILE_NOT_FOUND => return Ok(None),
        code => return Err(registry_error(&format!("{subkey}\\{value}"), code)),
    }

    // `size` is in bytes and includes the terminating NUL.
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey_w.as_ptr()),
            PCWSTR(value_w.as_ptr()),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    match status {
        ERROR_SUCCESS => {}
        ERROR_FILE_NOT_FOUND => return Ok(None),
        code => return Err(registry_error(&format!("{subkey}\\{value}"), code)),
    }

    // Trim at the first NUL; the API may report a larger buffer than it filled.
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Ok(Some(String::from_utf16_lossy(&buffer[..end])))
}

/// Read a `REG_DWORD` value. `Ok(None)` means absent.
pub fn read_u32(root: HKEY, subkey: &str, value: &str) -> Result<Option<u32>> {
    let subkey_w = HSTRING::from(subkey);
    let value_w = HSTRING::from(value);

    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey_w.as_ptr()),
            PCWSTR(value_w.as_ptr()),
            RRF_RT_REG_DWORD | RRF_SUBKEY_WOW6464KEY,
            None,
            Some(std::ptr::addr_of_mut!(data).cast()),
            Some(&mut size),
        )
    };
    match status {
        ERROR_SUCCESS => Ok(Some(data)),
        ERROR_FILE_NOT_FOUND => Ok(None),
        code => Err(registry_error(&format!("{subkey}\\{value}"), code)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CurrentBuildNumber` exists on every supported Windows version, so this
    /// exercises the real API without depending on anything we installed.
    #[test]
    fn reads_a_well_known_string_value() {
        let build = read_string(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "CurrentBuildNumber",
        )
        .expect("read should succeed")
        .expect("CurrentBuildNumber exists on all supported Windows versions");

        assert!(
            build.parse::<u32>().is_ok(),
            "expected a numeric build, got {build:?}"
        );
    }

    #[test]
    fn missing_value_is_none_not_an_error() {
        let result = read_string(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "SioDefinitelyDoesNotExist",
        );
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn missing_key_is_none_not_an_error() {
        let result = read_string(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Sio\NoSuchKey\AtAll",
            "Anything",
        );
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn reads_a_dword() {
        // CurrentMajorVersionNumber is a DWORD present on Windows 10 and 11.
        let major = read_u32(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "CurrentMajorVersionNumber",
        )
        .expect("read should succeed");
        assert_eq!(
            major,
            Some(10),
            "both Windows 10 and 11 report major version 10"
        );
    }

    #[test]
    fn hive_mapping_is_stable() {
        assert_eq!(hkey_for(Hive::Hklm), HKEY_LOCAL_MACHINE);
        assert_eq!(hkey_for(Hive::Hkcu), HKEY_CURRENT_USER);
    }
}
