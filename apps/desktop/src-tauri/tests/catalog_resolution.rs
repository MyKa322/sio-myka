//! Resolves the shipped catalog against the *real* package managers on this machine.
//!
//! The unit tests cover catalog validation and provider parsing separately. This joins
//! them: it detects what is actually installed on this box and checks that every
//! catalog entry resolves to something installable, which is what the Apps screen shows.
//!
//! It runs real `winget export` and `choco list` commands, so it is slower than a unit
//! test, but it is the check that would have caught a catalog full of plausible-looking
//! ids that no provider can actually resolve.

#![cfg(windows)]

use sio_core::catalog::AppCatalog;
use sio_core::package::ProviderId;
use sio_packages::ProviderRegistry;

const APPS_JSON: &str = include_str!("../../../../catalog/apps.json");

fn catalog() -> AppCatalog {
    AppCatalog::from_json(APPS_JSON).expect("the shipped catalog must be valid")
}

/// Whether this machine can support the real-provider checks.
///
/// winget ships with Windows 11 but is a Store app, and CI runners do not reliably
/// have it. Skipping there beats a flaky red build — these tests earn their keep on a
/// developer machine, where the catalog is actually edited.
async fn registry_or_skip() -> Option<ProviderRegistry> {
    let registry = ProviderRegistry::detect().await;
    if registry.available().contains(&ProviderId::Winget) {
        return Some(registry);
    }
    println!(
        "skipping: winget is not available here (detected {:?})",
        registry.available()
    );
    None
}

#[tokio::test]
async fn every_catalog_app_resolves_to_an_installable_source() {
    let Some(registry) = registry_or_skip().await else {
        return;
    };
    let available = registry.available().to_vec();

    let unresolvable: Vec<_> = catalog()
        .apps
        .iter()
        .filter(|app| app.preferred_source(&available).is_none())
        .map(|app| app.id.clone())
        .collect();

    assert!(
        unresolvable.is_empty(),
        "these catalog apps cannot be installed with {available:?}: {unresolvable:#?}"
    );
}

#[tokio::test]
async fn the_installed_flag_agrees_with_the_provider_inventory() {
    // The Apps screen greys out anything already present. That flag comes from the
    // provider inventories, so it must agree with them exactly — a mismatch means
    // either a false "Installed" badge or an app the user cannot select.
    let Some(registry) = registry_or_skip().await else {
        return;
    };
    let catalog = catalog();

    let mut installed_count = 0;
    for app in &catalog.apps {
        let flagged = app
            .sources
            .iter()
            .any(|s| registry.is_installed(s.provider, &s.id));

        let from_inventory = app.sources.iter().any(|s| {
            registry
                .inventory()
                .iter()
                .any(|p| p.provider == s.provider && p.id.eq_ignore_ascii_case(&s.id))
        });

        assert_eq!(
            flagged, from_inventory,
            "`{}` disagrees with the inventory",
            app.id
        );
        if flagged {
            installed_count += 1;
        }
    }

    // Not asserted as a minimum: a clean CI runner legitimately has none of them.
    println!(
        "{installed_count} of {} catalog apps are installed here",
        catalog.apps.len()
    );
}

#[tokio::test]
async fn detection_reports_a_non_empty_inventory_when_winget_is_present() {
    // `winget export` returning nothing on a real desktop would mean the inventory
    // parsing silently broke — the failure mode that makes every app look uninstalled.
    let Some(registry) = registry_or_skip().await else {
        return;
    };

    assert!(
        !registry.inventory().is_empty(),
        "winget is present but reported no installed packages at all"
    );
}
