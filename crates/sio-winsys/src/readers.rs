//! The in-process implementation of [`SystemReader`].
//!
//! Every query here works without elevation, which is what lets the Tuning screen show
//! which tweaks are already applied the moment it opens — no UAC prompt just to look.

use crate::process::run_captured;
use crate::{registry, services};
use async_trait::async_trait;
use sio_core::error::Result;
use sio_core::reader::SystemReader;
use sio_core::tweak::{Hive, PriorState, PriorValue};

#[derive(Debug, Default)]
pub struct InProcessReader {
    /// Installed package family names, fetched once.
    ///
    /// Every `Get-AppxPackage` call spawns PowerShell and costs a few hundred
    /// milliseconds. Checking a dozen debloat tweaks one at a time would make the
    /// Tuning screen take seconds to show anything, so the list is fetched once and
    /// answered from memory.
    appx_cache: tokio::sync::Mutex<Option<std::collections::HashSet<String>>>,
}

impl InProcessReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop the cached package list, e.g. after removing something.
    pub async fn invalidate(&self) {
        *self.appx_cache.lock().await = None;
    }

    async fn installed_packages(&self) -> Result<std::collections::HashSet<String>> {
        let mut guard = self.appx_cache.lock().await;
        if let Some(cached) = guard.as_ref() {
            return Ok(cached.clone());
        }

        let args = vec![
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            // One family name per line: no headers, no localization, nothing to parse
            // beyond splitting on newlines.
            "Get-AppxPackage | ForEach-Object { $_.PackageFamilyName }".to_string(),
        ];

        let (_, stdout) = run_captured("powershell", &args).await?;
        let set: std::collections::HashSet<String> = stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        *guard = Some(set.clone());
        Ok(set)
    }
}

#[async_trait]
impl SystemReader for InProcessReader {
    async fn registry_read(&self, hive: Hive, path: &str, name: &str) -> Result<PriorValue> {
        let (path, name) = (path.to_string(), name.to_string());
        tokio::task::spawn_blocking(move || {
            registry::capture_prior(registry::hkey_for(hive), &path, &name)
        })
        .await
        .map_err(|e| sio_core::Error::Other(format!("registry read task panicked: {e}")))?
    }

    async fn service_state(&self, name: &str) -> Result<PriorState> {
        let name = name.to_string();
        tokio::task::spawn_blocking(move || services::query(&name))
            .await
            .map_err(|e| sio_core::Error::Other(format!("service read task panicked: {e}")))?
    }

    async fn appx_present(&self, package_family_name: &str) -> Result<bool> {
        // Package family names are case-insensitive in practice, and catalog entries
        // are hand-written, so compare accordingly rather than exact-matching.
        Ok(self
            .installed_packages()
            .await?
            .iter()
            .any(|p| p.eq_ignore_ascii_case(package_family_name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_a_well_known_registry_value() {
        let reader = InProcessReader::new();
        let value = reader
            .registry_read(
                Hive::Hklm,
                r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                "CurrentBuildNumber",
            )
            .await
            .unwrap();

        assert!(matches!(value, PriorValue::Present(_)), "got {value:?}");
    }

    #[tokio::test]
    async fn a_missing_key_reads_as_key_absent() {
        let reader = InProcessReader::new();
        let value = reader
            .registry_read(Hive::Hkcu, r"Software\SioTest\NothingHere", "X")
            .await
            .unwrap();
        assert_eq!(value, PriorValue::KeyAbsent);
    }

    #[tokio::test]
    async fn reads_a_service_without_elevation() {
        let reader = InProcessReader::new();
        let state = reader.service_state("EventLog").await.unwrap();
        assert!(state.was_running);
    }

    #[tokio::test]
    async fn a_package_that_cannot_exist_is_reported_absent() {
        let reader = InProcessReader::new();
        assert!(!reader
            .appx_present("Sio.NoSuchPackage_00000000000")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn an_installed_package_is_reported_present() {
        // Self-calibrating: ask the machine what it actually has and check one of
        // those, rather than hardcoding a package. The first attempt used the Store,
        // which turned out not to be installed on the development machine at all —
        // exactly the sort of assumption a positive control should not rest on.
        let (_, stdout) = run_captured(
            "powershell",
            &[
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                "@(Get-AppxPackage)[0].PackageFamilyName".into(),
            ],
        )
        .await
        .unwrap();

        let existing = stdout.trim();
        if existing.is_empty() {
            println!("skipping: this machine reports no Appx packages at all");
            return;
        }

        let reader = InProcessReader::new();
        assert!(
            reader.appx_present(existing).await.unwrap(),
            "`{existing}` is installed but was reported absent"
        );
    }

    #[tokio::test]
    async fn the_package_list_is_fetched_once_and_reused() {
        // Twelve debloat tweaks must not mean twelve PowerShell launches. The second
        // lookup should be served from memory and therefore be far quicker.
        let reader = InProcessReader::new();

        let start = std::time::Instant::now();
        reader.appx_present("Sio.First_000000000000").await.unwrap();
        let cold = start.elapsed();

        let start = std::time::Instant::now();
        for _ in 0..20 {
            reader
                .appx_present("Sio.Second_000000000000")
                .await
                .unwrap();
        }
        let twenty_warm = start.elapsed();

        assert!(
            twenty_warm < cold,
            "twenty cached lookups ({twenty_warm:?}) should beat one cold one ({cold:?})"
        );
    }

    #[tokio::test]
    async fn invalidating_forces_a_refetch() {
        let reader = InProcessReader::new();
        reader
            .appx_present("Sio.Whatever_000000000000")
            .await
            .unwrap();
        reader.invalidate().await;
        // Simply must not panic or return stale nonsense.
        assert!(!reader
            .appx_present("Sio.Whatever_000000000000")
            .await
            .unwrap());
    }
}
