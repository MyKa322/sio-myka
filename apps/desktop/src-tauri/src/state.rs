//! Application state: the catalogs and the detected package managers.
//!
//! Provider detection runs three executables and reads three inventories, which costs a
//! second or two. It happens once, on first use, and is cached — doing it per request
//! would make every screen feel sluggish.

use sio_core::catalog::{AppCatalog, TweakCatalog};
use sio_core::error::Result;
use sio_packages::ProviderRegistry;
use sio_tweaks::JournalStore;
use sio_winsys::InProcessReader;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct AppState {
    apps: AppCatalog,
    tweaks: TweakCatalog,
    providers: RwLock<Option<Arc<ProviderRegistry>>>,
    journal: JournalStore,
    reader: InProcessReader,
    /// Windows build number, used to hide tweaks that do not apply to this version.
    os_build: u32,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("apps", &self.apps.apps.len())
            .field("tweaks", &self.tweaks.tweaks.len())
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Load the bundled catalogs.
    ///
    /// A failure here means the binary shipped a catalog it cannot parse, which CI
    /// should have caught — so it is a hard error rather than a silent empty list.
    pub fn load() -> Result<Self> {
        Ok(Self {
            apps: crate::catalog::load_apps()?,
            tweaks: crate::catalog::load_tweaks()?,
            providers: RwLock::new(None),
            journal: JournalStore::open_default()?,
            reader: InProcessReader::new(),
            os_build: current_build(),
        })
    }

    pub fn journal(&self) -> &JournalStore {
        &self.journal
    }

    pub fn reader(&self) -> &InProcessReader {
        &self.reader
    }

    /// Only the tests care about the raw number; production code goes through
    /// [`Self::applicable_tweaks`], which already applies it.
    #[cfg(test)]
    pub fn os_build(&self) -> u32 {
        self.os_build
    }

    /// Tweaks that apply to this Windows version.
    ///
    /// Showing a Windows 11 taskbar tweak to a Windows 10 user would be a setting that
    /// silently does nothing.
    pub fn applicable_tweaks(&self) -> impl Iterator<Item = &sio_core::tweak::Tweak> {
        self.tweaks.for_build(self.os_build)
    }

    pub fn apps(&self) -> &AppCatalog {
        &self.apps
    }

    #[cfg(test)]
    pub fn tweak_count(&self) -> usize {
        self.tweaks.tweaks.len()
    }

    /// Detected package managers, probing on first call.
    pub async fn providers(&self) -> Arc<ProviderRegistry> {
        if let Some(existing) = self.providers.read().await.as_ref() {
            return Arc::clone(existing);
        }

        let mut guard = self.providers.write().await;
        // Another task may have won the race between the read and write locks.
        if let Some(existing) = guard.as_ref() {
            return Arc::clone(existing);
        }

        let registry = Arc::new(ProviderRegistry::detect().await);
        *guard = Some(Arc::clone(&registry));
        registry
    }

    /// Re-probe, e.g. after the user installs Chocolatey or Scoop, or after a batch
    /// install changes what is present.
    pub async fn refresh_providers(&self) -> Arc<ProviderRegistry> {
        let registry = Arc::new(ProviderRegistry::detect().await);
        *self.providers.write().await = Some(Arc::clone(&registry));
        registry
    }
}

/// Read the Windows build number. Zero if it cannot be read, which makes
/// `Applies::Windows11Only` tweaks hide rather than appear wrongly.
fn current_build() -> u32 {
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

    sio_winsys::registry::read_string(
        HKEY_LOCAL_MACHINE,
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "CurrentBuildNumber",
    )
    .ok()
    .flatten()
    .and_then(|s| s.parse().ok())
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_loads_the_bundled_catalogs() {
        let state = AppState::load().expect("bundled catalogs must load");
        assert!(!state.apps().apps.is_empty());
        assert!(state.tweak_count() > 0);
    }

    #[test]
    fn the_build_number_is_read_and_filters_the_tweak_list() {
        let state = AppState::load().unwrap();
        assert!(
            state.os_build() >= 10240,
            "expected a real build, got {}",
            state.os_build()
        );

        // Every shipped tweak is either universal or matches this machine's version;
        // none should be advertised that would silently do nothing here.
        let applicable = state.applicable_tweaks().count();
        assert!(applicable > 0);
        assert!(applicable <= state.tweak_count());
    }

    #[tokio::test]
    async fn provider_detection_is_cached() {
        let state = AppState::load().unwrap();

        let first = state.providers().await;
        let second = state.providers().await;

        // Same allocation, so detection ran once rather than per call.
        assert!(Arc::ptr_eq(&first, &second), "detection must be cached");
    }
}
