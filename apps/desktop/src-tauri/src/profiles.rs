//! Saved profiles on disk.
//!
//! A profile is the thing you actually carry to a fresh install: a list of catalog ids,
//! not resolved versions or command lines, so one saved a year ago still installs
//! current software today.
//!
//! Stored under `%APPDATA%\SIO\profiles` as plain JSON — readable, diffable, and easy
//! to copy to a USB stick before wiping the machine.

use sio_core::error::{Error, Result};
use sio_core::profile::Profile;
use std::path::{Path, PathBuf};

/// Where profiles live.
pub fn profiles_dir() -> Result<PathBuf> {
    let base =
        std::env::var_os("APPDATA").ok_or_else(|| Error::Other("APPDATA is not set".into()))?;
    Ok(PathBuf::from(base).join("SIO").join("profiles"))
}

/// Turn a user-supplied profile name into a safe file name.
///
/// Profile names come from a text box, so they can contain path separators, reserved
/// Windows device names, or nothing at all. Without this, "../../evil" would write
/// outside the profiles directory.
pub fn safe_file_name(name: &str) -> String {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    let cleaned: String = name
        .chars()
        .map(|c| match c {
            // Everything Windows forbids in a file name, plus the path separators.
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();

    // Windows silently strips trailing dots and spaces, which would make "a." and "a"
    // collide and overwrite each other.
    let trimmed = cleaned.trim().trim_end_matches('.').trim();

    if trimmed.is_empty() {
        return "profile".to_string();
    }

    let stem_upper = trimmed.split('.').next().unwrap_or(trimmed).to_uppercase();
    if RESERVED.contains(&stem_upper.as_str()) {
        return format!("{trimmed}_");
    }

    trimmed.to_string()
}

fn path_for(name: &str) -> Result<PathBuf> {
    Ok(profiles_dir()?.join(format!("{}.json", safe_file_name(name))))
}

/// Write a profile, creating the directory if needed.
pub async fn save(profile: &Profile) -> Result<PathBuf> {
    let dir = profiles_dir()?;
    tokio::fs::create_dir_all(&dir).await?;

    let path = path_for(&profile.name)?;
    let json = serde_json::to_string_pretty(profile)?;
    tokio::fs::write(&path, json).await?;
    Ok(path)
}

/// Read every profile in the directory.
///
/// A single unreadable or malformed file is skipped with a warning rather than failing
/// the whole listing — one corrupt profile should not hide the other five.
pub async fn list() -> Result<Vec<Profile>> {
    let dir = profiles_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = tokio::fs::read_dir(&dir).await?;
    let mut profiles = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => match Profile::from_json(&text) {
                Ok(profile) => profiles.push(profile),
                Err(e) => tracing::warn!("skipping malformed profile {}: {e}", path.display()),
            },
            Err(e) => tracing::warn!("could not read {}: {e}", path.display()),
        }
    }

    // Newest first — the profile you just saved is the one you want to see.
    profiles.sort_by_key(|p| std::cmp::Reverse(p.created_at));
    Ok(profiles)
}

/// Delete a profile by name. Missing is success.
pub async fn delete(name: &str) -> Result<()> {
    let path = path_for(name)?;
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Show the profiles folder in Explorer.
///
/// Profiles are plain JSON in a known place, so "export" and "import" are just copying
/// a file. Opening the folder does that job without pulling in a file-dialog plugin,
/// and it also lets someone drop a profile they carried over on a USB stick.
pub async fn reveal_folder() -> Result<()> {
    let dir = profiles_dir()?;
    tokio::fs::create_dir_all(&dir).await?;
    open_in_explorer(&dir).await
}

async fn open_in_explorer(path: &Path) -> Result<()> {
    // explorer.exe returns a non-zero exit code even when it succeeds, so the status
    // is deliberately ignored — only a spawn failure is a real error here.
    tokio::process::Command::new("explorer")
        .arg(path)
        .spawn()
        .map_err(|e| Error::Other(format!("could not open {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_traversal_name_still_writes_inside_the_profiles_directory() {
        // The invariant that matters is containment, not the shape of the string.
        // "../../evil" sanitises to ".._.._evil", which keeps the dots but has no path
        // separators, so it is just an oddly-named file in the right folder.
        for hostile in [
            "../../evil",
            r"..\..\evil",
            "/etc/passwd",
            r"C:\Windows\System32\x",
        ] {
            let path = path_for(hostile).expect("APPDATA should be set");
            assert_eq!(
                path.parent(),
                Some(profiles_dir().unwrap().as_path()),
                "{hostile:?} escaped to {}",
                path.display()
            );
        }
    }

    #[test]
    fn sanitised_names_never_contain_a_path_separator() {
        for hostile in ["../../evil", r"a\b", "a/b"] {
            let name = safe_file_name(hostile);
            assert!(!name.contains('/') && !name.contains('\\'), "got {name}");
        }
    }

    #[test]
    fn a_bare_dot_name_does_not_become_a_directory_reference() {
        // ".." must not survive as a name that means "the parent directory".
        assert_ne!(safe_file_name(".."), "..");
        assert_ne!(safe_file_name("."), ".");
    }

    #[test]
    fn characters_windows_forbids_are_replaced() {
        assert_eq!(safe_file_name(r#"a<b>c:d"e|f?g*h"#), "a_b_c_d_e_f_g_h");
    }

    #[test]
    fn reserved_device_names_are_suffixed() {
        // A file literally named CON cannot be created on Windows.
        assert_eq!(safe_file_name("CON"), "CON_");
        assert_eq!(safe_file_name("con"), "con_");
        assert_eq!(safe_file_name("LPT1"), "LPT1_");
        assert_eq!(safe_file_name("COM3.json"), "COM3.json_");
    }

    #[test]
    fn ordinary_names_are_left_readable() {
        assert_eq!(safe_file_name("Gaming PC"), "Gaming PC");
        assert_eq!(safe_file_name("Ноутбук"), "Ноутбук");
        assert_eq!(safe_file_name("Робочий ПК"), "Робочий ПК");
    }

    #[test]
    fn trailing_dots_and_spaces_are_trimmed() {
        // Windows strips these itself, so "Work." and "Work" would collide.
        assert_eq!(safe_file_name("Work."), "Work");
        assert_eq!(safe_file_name("  Work  "), "Work");
        assert_eq!(safe_file_name("Work..."), "Work");
    }

    #[test]
    fn an_empty_or_whitespace_name_gets_a_fallback() {
        assert_eq!(safe_file_name(""), "profile");
        assert_eq!(safe_file_name("   "), "profile");
        assert_eq!(safe_file_name("..."), "profile");
    }

    #[tokio::test]
    async fn save_list_and_delete_round_trip() {
        let name = format!("SioUnitTest-{}", std::process::id());
        let mut profile = Profile::new(&name, sio_core::now_unix_ms());
        profile.apps = vec!["firefox".into(), "7zip".into()];
        profile.tweaks = vec!["privacy.telemetry.disable".into()];

        save(&profile).await.expect("save should succeed");

        let all = list().await.expect("list should succeed");
        let found = all
            .iter()
            .find(|p| p.name == name)
            .expect("the saved profile should appear");
        assert_eq!(found.apps, profile.apps);
        assert_eq!(found.tweaks, profile.tweaks);

        delete(&name).await.expect("delete should succeed");
        let after = list().await.unwrap();
        assert!(!after.iter().any(|p| p.name == name));
    }

    #[tokio::test]
    async fn deleting_a_missing_profile_is_not_an_error() {
        assert!(delete("SioNoSuchProfileAnywhere").await.is_ok());
    }
}
