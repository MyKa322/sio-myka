//! The set of package managers usable on this machine.
//!
//! Detection is done once and cached. Probing availability and reading three
//! inventories costs a couple of seconds — acceptable at startup, not acceptable per
//! app in a thirty-item batch.

use crate::chocolatey::Chocolatey;
use crate::provider::PackageProvider;
use crate::scoop::Scoop;
use crate::winget::Winget;
use sio_core::package::{InstalledPackage, ProviderId};
use std::collections::HashSet;

/// A provider plus a lowercased package id, used for "is this already installed?".
///
/// Lowercased because Chocolatey and Scoop are case-insensitive while catalog entries
/// are written however the publisher spells them.
pub type InstalledKey = (ProviderId, String);

pub struct ProviderRegistry {
    providers: Vec<Box<dyn PackageProvider>>,
    available: Vec<ProviderId>,
    installed: HashSet<InstalledKey>,
    inventory: Vec<InstalledPackage>,
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("available", &self.available)
            .field("installed", &self.installed.len())
            .finish_non_exhaustive()
    }
}

impl ProviderRegistry {
    /// Probe the machine: which managers exist, and what they already have installed.
    pub async fn detect() -> Self {
        let providers: Vec<Box<dyn PackageProvider>> = vec![
            Box::new(Winget::new()),
            Box::new(Chocolatey::new()),
            Box::new(Scoop::new()),
        ];

        let mut available = Vec::new();
        let mut inventory = Vec::new();

        for provider in &providers {
            if !provider.is_available().await {
                tracing::debug!("{} is not available", provider.id());
                continue;
            }
            available.push(provider.id());

            // A provider that is present but whose inventory cannot be read should not
            // take down the others — it just means we cannot skip its already-installed
            // packages, and the package manager will tell us itself.
            match provider.installed().await {
                Ok(packages) => {
                    tracing::debug!(
                        "{} reports {} installed packages",
                        provider.id(),
                        packages.len()
                    );
                    inventory.extend(packages);
                }
                Err(e) => tracing::warn!("could not read the {} inventory: {e}", provider.id()),
            }
        }

        let installed = inventory
            .iter()
            .map(|p| (p.provider, p.id.to_lowercase()))
            .collect();

        Self {
            providers,
            available,
            installed,
            inventory,
        }
    }

    /// Construct directly. For tests.
    pub fn from_parts(
        providers: Vec<Box<dyn PackageProvider>>,
        available: Vec<ProviderId>,
        installed: Vec<InstalledKey>,
    ) -> Self {
        Self {
            providers,
            available,
            installed: installed.into_iter().collect(),
            inventory: Vec::new(),
        }
    }

    pub fn get(&self, id: ProviderId) -> Option<&dyn PackageProvider> {
        if !self.available.contains(&id) {
            return None;
        }
        self.providers
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
    }

    pub fn available(&self) -> &[ProviderId] {
        &self.available
    }

    pub fn installed_ids(&self) -> &HashSet<InstalledKey> {
        &self.installed
    }

    pub fn inventory(&self) -> &[InstalledPackage] {
        &self.inventory
    }

    /// Whether a specific package is already present.
    pub fn is_installed(&self, provider: ProviderId, id: &str) -> bool {
        self.installed.contains(&(provider, id.to_lowercase()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> ProviderRegistry {
        ProviderRegistry::from_parts(
            vec![Box::new(Winget::new()), Box::new(Chocolatey::new())],
            vec![ProviderId::Winget],
            vec![(ProviderId::Winget, "mozilla.firefox".into())],
        )
    }

    #[test]
    fn get_returns_only_available_providers() {
        let registry = registry();
        assert!(registry.get(ProviderId::Winget).is_some());

        // Chocolatey is constructed but not marked available, so it must not be handed
        // out — otherwise a bulk install would invoke a manager that is not installed.
        assert!(registry.get(ProviderId::Chocolatey).is_none());
        assert!(registry.get(ProviderId::Scoop).is_none());
    }

    #[test]
    fn installed_lookup_ignores_case() {
        let registry = registry();
        assert!(registry.is_installed(ProviderId::Winget, "Mozilla.Firefox"));
        assert!(registry.is_installed(ProviderId::Winget, "mozilla.firefox"));
        assert!(!registry.is_installed(ProviderId::Winget, "Mozilla.Thunderbird"));
    }

    #[test]
    fn the_same_id_under_a_different_provider_is_not_a_match() {
        // "firefox" on Chocolatey is not "Mozilla.Firefox" on winget.
        let registry = registry();
        assert!(!registry.is_installed(ProviderId::Chocolatey, "mozilla.firefox"));
    }

    /// Real detection against this machine. winget ships with Windows, so it must be
    /// found; the rest depends on the box.
    #[tokio::test]
    async fn detect_finds_winget_on_a_real_windows_machine() {
        let registry = ProviderRegistry::detect().await;
        assert!(
            registry.available().contains(&ProviderId::Winget),
            "winget ships with Windows 11; got {:?}",
            registry.available()
        );
        assert!(registry.get(ProviderId::Winget).is_some());
    }
}
