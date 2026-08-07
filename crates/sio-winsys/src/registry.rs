//! Registry access.
//!
//! Safe wrappers over the Win32 registry APIs. Reads back the dashboard's OS facts, and
//! provides the read-then-write primitive the reversible tweak engine is built on.
//!
//! The critical operation here is [`capture_prior`], which distinguishes three states
//! that Win32 reports almost identically: the value exists, the value is missing, and
//! the whole key is missing. Reverting the second two means *deleting*, and writing a
//! plausible default instead is how a tuning tool quietly corrupts a machine.

use sio_core::error::{Error, Result};
use sio_core::tweak::{Hive, PriorValue, RegistryValue};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
    HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_READ,
    KEY_SET_VALUE, REG_BINARY, REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ, REG_OPTION_NON_VOLATILE,
    REG_QWORD, REG_SZ, REG_VALUE_TYPE, RRF_NOEXPAND, RRF_RT_ANY, RRF_SUBKEY_WOW6464KEY,
};

pub fn hkey_for(hive: Hive) -> HKEY {
    match hive {
        Hive::Hklm => HKEY_LOCAL_MACHINE,
        Hive::Hkcu => HKEY_CURRENT_USER,
        Hive::Hkcr => HKEY_CLASSES_ROOT,
        Hive::Hku => HKEY_USERS,
    }
}

fn err(path: &str, api: &str, code: WIN32_ERROR) -> Error {
    Error::Registry {
        path: path.to_string(),
        reason: format!("{api} failed with win32 error {}", code.0),
    }
}

/// RAII guard so an early return can't leak a registry handle.
struct OwnedKey(HKEY);

impl Drop for OwnedKey {
    fn drop(&mut self) {
        // Nothing useful to do if closing fails, and it must not mask the real error.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// Whether a subkey exists at all.
pub fn key_exists(root: HKEY, subkey: &str) -> Result<bool> {
    match open_key(root, subkey, KEY_READ) {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(e) => Err(e),
    }
}

fn open_key(
    root: HKEY,
    subkey: &str,
    access: windows::Win32::System::Registry::REG_SAM_FLAGS,
) -> Result<Option<OwnedKey>> {
    let subkey_w = HSTRING::from(subkey);
    let mut handle = HKEY::default();
    let status =
        unsafe { RegOpenKeyExW(root, PCWSTR(subkey_w.as_ptr()), None, access, &mut handle) };
    match status {
        ERROR_SUCCESS => Ok(Some(OwnedKey(handle))),
        ERROR_FILE_NOT_FOUND => Ok(None),
        code => Err(err(subkey, "RegOpenKeyExW", code)),
    }
}

/// Read a value's raw type and bytes.
///
/// `RRF_NOEXPAND` matters: without it Win32 expands `REG_EXPAND_SZ` values, and we
/// would capture `C:\Users\Bob\AppData` where the registry actually held
/// `%USERPROFILE%\AppData`. Reverting would then hard-code one machine's paths.
fn read_raw(root: HKEY, subkey: &str, name: &str) -> Result<Option<(REG_VALUE_TYPE, Vec<u8>)>> {
    let subkey_w = HSTRING::from(subkey);
    let name_w = HSTRING::from(name);
    let flags = RRF_RT_ANY | RRF_NOEXPAND | RRF_SUBKEY_WOW6464KEY;

    let mut kind = REG_VALUE_TYPE::default();
    let mut size: u32 = 0;
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey_w.as_ptr()),
            PCWSTR(name_w.as_ptr()),
            flags,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };
    match status {
        ERROR_SUCCESS | ERROR_MORE_DATA => {}
        ERROR_FILE_NOT_FOUND => return Ok(None),
        code => return Err(err(&format!("{subkey}\\{name}"), "RegGetValueW", code)),
    }

    let mut buffer = vec![0u8; size as usize];
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey_w.as_ptr()),
            PCWSTR(name_w.as_ptr()),
            flags,
            Some(&mut kind),
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    match status {
        ERROR_SUCCESS => {}
        ERROR_FILE_NOT_FOUND => return Ok(None),
        code => return Err(err(&format!("{subkey}\\{name}"), "RegGetValueW", code)),
    }

    // The second call may report fewer bytes than the first reserved.
    buffer.truncate(size as usize);
    Ok(Some((kind, buffer)))
}

fn utf16_from_bytes(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn decode(kind: REG_VALUE_TYPE, bytes: &[u8]) -> Option<RegistryValue> {
    let string_of = |b: &[u8]| {
        let units = utf16_from_bytes(b);
        let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    };

    Some(match kind {
        REG_DWORD => RegistryValue::Dword(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?)),
        REG_QWORD => RegistryValue::Qword(u64::from_le_bytes(bytes.get(..8)?.try_into().ok()?)),
        REG_SZ => RegistryValue::String(string_of(bytes)),
        REG_EXPAND_SZ => RegistryValue::ExpandString(string_of(bytes)),
        REG_MULTI_SZ => {
            let units = utf16_from_bytes(bytes);
            let parts = units
                .split(|&c| c == 0)
                .filter(|s| !s.is_empty())
                .map(String::from_utf16_lossy)
                .collect();
            RegistryValue::MultiString(parts)
        }
        REG_BINARY => RegistryValue::Binary(bytes.to_vec()),
        // An exotic type (REG_LINK, REG_RESOURCE_LIST) we cannot faithfully rewrite.
        _ => return None,
    })
}

fn encode(value: &RegistryValue) -> (REG_VALUE_TYPE, Vec<u8>) {
    fn wide_nul(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    match value {
        RegistryValue::Dword(v) => (REG_DWORD, v.to_le_bytes().to_vec()),
        RegistryValue::Qword(v) => (REG_QWORD, v.to_le_bytes().to_vec()),
        RegistryValue::String(s) => (REG_SZ, wide_nul(s)),
        RegistryValue::ExpandString(s) => (REG_EXPAND_SZ, wide_nul(s)),
        RegistryValue::MultiString(parts) => {
            // REG_MULTI_SZ is NUL-separated and NUL-terminated, so an empty list is
            // still a single terminating NUL rather than nothing at all.
            let mut bytes: Vec<u8> = parts.iter().flat_map(|p| wide_nul(p)).collect();
            bytes.extend_from_slice(&0u16.to_le_bytes());
            (REG_MULTI_SZ, bytes)
        }
        RegistryValue::Binary(b) => (REG_BINARY, b.clone()),
    }
}

/// Read a typed value. `Ok(None)` means absent or an unrepresentable type.
pub fn read_value(root: HKEY, subkey: &str, name: &str) -> Result<Option<RegistryValue>> {
    Ok(read_raw(root, subkey, name)?.and_then(|(kind, bytes)| decode(kind, &bytes)))
}

/// Read a value as text. `Ok(None)` if absent or not a string type.
///
/// Convenience for the dashboard, which only reads `REG_SZ` facts about the OS.
pub fn read_string(root: HKEY, subkey: &str, name: &str) -> Result<Option<String>> {
    Ok(match read_value(root, subkey, name)? {
        Some(RegistryValue::String(s) | RegistryValue::ExpandString(s)) => Some(s),
        _ => None,
    })
}

/// Capture the exact current state of a location, ready to be journalled.
///
/// This is the function the whole revert guarantee rests on.
pub fn capture_prior(root: HKEY, subkey: &str, name: &str) -> Result<PriorValue> {
    if !key_exists(root, subkey)? {
        return Ok(PriorValue::KeyAbsent);
    }
    match read_value(root, subkey, name)? {
        Some(value) => Ok(PriorValue::Present(value)),
        None => Ok(PriorValue::Absent),
    }
}

/// Write a value, creating the key if it does not exist.
pub fn write_value(root: HKEY, subkey: &str, name: &str, value: &RegistryValue) -> Result<()> {
    let subkey_w = HSTRING::from(subkey);
    let name_w = HSTRING::from(name);
    let (kind, bytes) = encode(value);

    let mut handle = HKEY::default();
    let status = unsafe {
        RegCreateKeyExW(
            root,
            PCWSTR(subkey_w.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut handle,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(err(subkey, "RegCreateKeyExW", status));
    }
    let key = OwnedKey(handle);

    let status =
        unsafe { RegSetValueExW(key.0, PCWSTR(name_w.as_ptr()), None, kind, Some(&bytes)) };
    if status != ERROR_SUCCESS {
        return Err(err(&format!("{subkey}\\{name}"), "RegSetValueExW", status));
    }
    Ok(())
}

/// Delete a value. Already-absent is success, not an error — revert must be idempotent
/// so a partially-applied tweak can be rolled back safely.
pub fn delete_value(root: HKEY, subkey: &str, name: &str) -> Result<()> {
    let Some(key) = open_key(root, subkey, KEY_SET_VALUE)? else {
        return Ok(());
    };
    let name_w = HSTRING::from(name);
    let status = unsafe { RegDeleteValueW(key.0, PCWSTR(name_w.as_ptr())) };
    match status {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        code => Err(err(&format!("{subkey}\\{name}"), "RegDeleteValueW", code)),
    }
}

/// Put a location back to a captured state.
///
/// Note that both absent variants restore by deletion. The key itself is deliberately
/// left behind: other tweaks may share it, and removing a key we merely created a value
/// under could take unrelated settings with it.
pub fn restore(root: HKEY, subkey: &str, name: &str, prior: &PriorValue) -> Result<()> {
    match prior {
        PriorValue::Absent | PriorValue::KeyAbsent => delete_value(root, subkey, name),
        PriorValue::Present(value) => write_value(root, subkey, name, value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scratch area under HKCU so tests need no elevation and touch nothing real.
    const TEST_KEY: &str = r"Software\SioTest\RegistryUnitTests";

    fn cleanup(name: &str) {
        let _ = delete_value(HKEY_CURRENT_USER, TEST_KEY, name);
    }

    #[test]
    fn reads_a_well_known_string_value() {
        let build = read_value(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "CurrentBuildNumber",
        )
        .expect("read should succeed")
        .expect("CurrentBuildNumber exists on all supported Windows versions");

        let RegistryValue::String(text) = build else {
            panic!("CurrentBuildNumber should be REG_SZ, got {build:?}");
        };
        assert!(
            text.parse::<u32>().is_ok(),
            "expected a numeric build, got {text:?}"
        );
    }

    #[test]
    fn missing_value_is_absent_but_missing_key_is_key_absent() {
        // The distinction the revert guarantee depends on.
        let key_absent = capture_prior(
            HKEY_CURRENT_USER,
            r"Software\SioTest\NoSuchKeyAtAll",
            "Whatever",
        )
        .unwrap();
        assert_eq!(key_absent, PriorValue::KeyAbsent);

        // Create the key by writing a value, then probe a *different*, absent name.
        write_value(
            HKEY_CURRENT_USER,
            TEST_KEY,
            "Anchor",
            &RegistryValue::Dword(1),
        )
        .unwrap();
        let value_absent = capture_prior(HKEY_CURRENT_USER, TEST_KEY, "NotSetHere").unwrap();
        assert_eq!(value_absent, PriorValue::Absent);
        cleanup("Anchor");
    }

    #[test]
    fn dword_round_trips() {
        let name = "DwordRoundTrip";
        write_value(
            HKEY_CURRENT_USER,
            TEST_KEY,
            name,
            &RegistryValue::Dword(0xDEAD_BEEF),
        )
        .unwrap();
        assert_eq!(
            read_value(HKEY_CURRENT_USER, TEST_KEY, name).unwrap(),
            Some(RegistryValue::Dword(0xDEAD_BEEF))
        );
        cleanup(name);
    }

    #[test]
    fn qword_and_binary_round_trip() {
        write_value(
            HKEY_CURRENT_USER,
            TEST_KEY,
            "Q",
            &RegistryValue::Qword(u64::MAX),
        )
        .unwrap();
        assert_eq!(
            read_value(HKEY_CURRENT_USER, TEST_KEY, "Q").unwrap(),
            Some(RegistryValue::Qword(u64::MAX))
        );

        let blob = RegistryValue::Binary(vec![0, 1, 2, 250, 255]);
        write_value(HKEY_CURRENT_USER, TEST_KEY, "B", &blob).unwrap();
        assert_eq!(
            read_value(HKEY_CURRENT_USER, TEST_KEY, "B").unwrap(),
            Some(blob)
        );

        cleanup("Q");
        cleanup("B");
    }

    #[test]
    fn strings_round_trip_including_non_ascii() {
        // The app targets Russian and Ukrainian users; UTF-16 handling must be right.
        let value = RegistryValue::String("Привіт, світ — ØÆ".into());
        write_value(HKEY_CURRENT_USER, TEST_KEY, "S", &value).unwrap();
        assert_eq!(
            read_value(HKEY_CURRENT_USER, TEST_KEY, "S").unwrap(),
            Some(value)
        );
        cleanup("S");
    }

    #[test]
    fn expand_strings_are_captured_unexpanded() {
        // Without RRF_NOEXPAND this would come back as the expanded path and revert
        // would bake one machine's directory layout into the registry.
        let value = RegistryValue::ExpandString("%USERPROFILE%\\SioTest".into());
        write_value(HKEY_CURRENT_USER, TEST_KEY, "E", &value).unwrap();

        let read = read_value(HKEY_CURRENT_USER, TEST_KEY, "E").unwrap();
        assert_eq!(
            read,
            Some(value),
            "the literal %USERPROFILE% must survive the round trip"
        );
        cleanup("E");
    }

    #[test]
    fn multi_strings_round_trip() {
        let value = RegistryValue::MultiString(vec!["one".into(), "two".into(), "три".into()]);
        write_value(HKEY_CURRENT_USER, TEST_KEY, "M", &value).unwrap();
        assert_eq!(
            read_value(HKEY_CURRENT_USER, TEST_KEY, "M").unwrap(),
            Some(value)
        );
        cleanup("M");
    }

    #[test]
    fn apply_then_revert_restores_a_previously_absent_value_by_deleting_it() {
        let name = "WasNeverThere";
        cleanup(name);
        // Anchor so the key itself exists; we want Absent, not KeyAbsent.
        write_value(
            HKEY_CURRENT_USER,
            TEST_KEY,
            "Anchor2",
            &RegistryValue::Dword(1),
        )
        .unwrap();

        let prior = capture_prior(HKEY_CURRENT_USER, TEST_KEY, name).unwrap();
        assert_eq!(prior, PriorValue::Absent);

        write_value(HKEY_CURRENT_USER, TEST_KEY, name, &RegistryValue::Dword(42)).unwrap();
        assert!(read_value(HKEY_CURRENT_USER, TEST_KEY, name)
            .unwrap()
            .is_some());

        restore(HKEY_CURRENT_USER, TEST_KEY, name, &prior).unwrap();
        assert_eq!(
            read_value(HKEY_CURRENT_USER, TEST_KEY, name).unwrap(),
            None,
            "reverting an absent value must delete it, not write a default"
        );
        cleanup("Anchor2");
    }

    #[test]
    fn apply_then_revert_restores_a_previous_value_exactly() {
        let name = "HadAValue";
        write_value(HKEY_CURRENT_USER, TEST_KEY, name, &RegistryValue::Dword(7)).unwrap();

        let prior = capture_prior(HKEY_CURRENT_USER, TEST_KEY, name).unwrap();
        assert_eq!(prior, PriorValue::Present(RegistryValue::Dword(7)));

        write_value(HKEY_CURRENT_USER, TEST_KEY, name, &RegistryValue::Dword(0)).unwrap();
        restore(HKEY_CURRENT_USER, TEST_KEY, name, &prior).unwrap();

        assert_eq!(
            read_value(HKEY_CURRENT_USER, TEST_KEY, name).unwrap(),
            Some(RegistryValue::Dword(7))
        );
        cleanup(name);
    }

    #[test]
    fn deleting_an_absent_value_succeeds_so_revert_is_idempotent() {
        assert!(delete_value(HKEY_CURRENT_USER, TEST_KEY, "NeverExisted").is_ok());
        assert!(delete_value(HKEY_CURRENT_USER, r"Software\SioTest\GoneKey", "X").is_ok());
    }

    #[test]
    fn writing_creates_missing_intermediate_keys() {
        let deep = r"Software\SioTest\A\B\C";
        write_value(HKEY_CURRENT_USER, deep, "Deep", &RegistryValue::Dword(1)).unwrap();
        assert_eq!(
            read_value(HKEY_CURRENT_USER, deep, "Deep").unwrap(),
            Some(RegistryValue::Dword(1))
        );
        let _ = delete_value(HKEY_CURRENT_USER, deep, "Deep");
    }

    #[test]
    fn hive_mapping_is_stable() {
        assert_eq!(hkey_for(Hive::Hklm), HKEY_LOCAL_MACHINE);
        assert_eq!(hkey_for(Hive::Hkcu), HKEY_CURRENT_USER);
    }
}
