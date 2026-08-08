//! Application state: the catalogs and the detected package managers.
//!
//! Provider detection runs three executables and reads three inventories, which costs a
//! second or two. It happens once, on first use, and is cached — doing it per request
//! would make every screen feel sluggish.

use sio_core::catalog::{AppCatalog, TweakCatalog};
use sio_core::error::Result;
use sio_packages::ProviderRegistry;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct AppState {
    apps: AppCatalog,
    tweaks: TweakCatalog,
    providers: RwLock<Option<Arc<ProviderRegistry>>>,
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
        })
    }

    pub fn apps(&self) -> &AppCatalog {
        &self.apps
    }

    /// Loaded and validated now so a malformed tweak catalog fails at startup rather
    /// than the first time someone opens the Tuning screen. Consumed in M4.
    #[allow(dead_code)]
    pub fn tweaks(&self) -> &TweakCatalog {
        &self.tweaks
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_loads_the_bundled_catalogs() {
        let state = AppState::load().expect("bundled catalogs must load");
        assert!(!state.apps().apps.is_empty());
        assert!(!state.tweaks().tweaks.is_empty());
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
