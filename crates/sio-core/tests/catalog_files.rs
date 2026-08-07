//! Validates the catalog files that actually ship.
//!
//! The unit tests in `catalog.rs` prove the rules work; this proves the checked-in data
//! obeys them. It runs in CI so a malformed entry fails review rather than landing on
//! someone's machine — the catalog is fetched at runtime, so a bad commit reaches users
//! without an app release to catch it.

use sio_core::catalog::{AppCatalog, TweakCatalog, Validate};
use sio_core::package::ProviderId;

const APPS_JSON: &str = include_str!("../../../catalog/apps.json");
const TWEAKS_JSON: &str = include_str!("../../../catalog/tweaks.json");

/// Every locale the app ships. A catalog entry missing one renders in the fallback
/// language, which looks broken next to a fully translated list.
const REQUIRED_LOCALES: [&str; 3] = ["en", "ru", "uk"];

fn apps() -> AppCatalog {
    AppCatalog::from_json(APPS_JSON).expect("catalog/apps.json must parse and validate")
}

fn tweaks() -> TweakCatalog {
    TweakCatalog::from_json(TWEAKS_JSON).expect("catalog/tweaks.json must parse and validate")
}

#[test]
fn app_catalog_is_valid() {
    let catalog = apps();
    catalog.validate().unwrap();
    assert!(
        !catalog.apps.is_empty(),
        "an empty catalog would ship a blank Apps screen"
    );
}

#[test]
fn tweak_catalog_is_valid() {
    let catalog = tweaks();
    catalog.validate().unwrap();
    assert!(!catalog.tweaks.is_empty());
}

#[test]
fn every_app_is_translated_into_all_shipped_locales() {
    let mut missing = Vec::new();
    for app in &apps().apps {
        for locale in REQUIRED_LOCALES {
            if !app.description.locales().any(|l| l == locale) {
                missing.push(format!("{}: {locale}", app.id));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "untranslated app descriptions: {missing:#?}"
    );
}

#[test]
fn every_tweak_name_is_translated_into_all_shipped_locales() {
    let mut missing = Vec::new();
    for tweak in &tweaks().tweaks {
        for locale in REQUIRED_LOCALES {
            if !tweak.name.locales().any(|l| l == locale) {
                missing.push(format!("{} name: {locale}", tweak.id));
            }
            if !tweak.description.is_empty() && !tweak.description.locales().any(|l| l == locale) {
                missing.push(format!("{} description: {locale}", tweak.id));
            }
        }
    }
    assert!(missing.is_empty(), "untranslated tweaks: {missing:#?}");
}

#[test]
fn every_app_is_installable_with_winget_alone() {
    // winget ships with Windows 11 and needs no bootstrap, so it is the only provider
    // guaranteed present. An app reachable only through Chocolatey or Scoop would show
    // as uninstallable on a clean machine, which defeats the point of the tool.
    let winget_only = [ProviderId::Winget];
    let unreachable: Vec<_> = apps()
        .apps
        .iter()
        .filter(|a| !a.is_installable(&winget_only))
        .map(|a| a.id.clone())
        .collect();

    assert!(
        unreachable.is_empty(),
        "not installable on a clean Windows 11: {unreachable:#?}"
    );
}

#[test]
fn no_app_lists_the_same_provider_twice() {
    for app in &apps().apps {
        let mut providers: Vec<_> = app.sources.iter().map(|s| s.provider).collect();
        let before = providers.len();
        providers.sort();
        providers.dedup();
        assert_eq!(
            before,
            providers.len(),
            "app `{}` lists a provider twice",
            app.id
        );
    }
}

#[test]
fn homepages_are_https() {
    for app in &apps().apps {
        if let Some(url) = &app.homepage {
            assert!(
                url.starts_with("https://"),
                "app `{}` has a non-https homepage: {url}",
                app.id
            );
        }
    }
}

#[test]
fn tweak_ids_are_dotted_and_lowercase() {
    // Ids appear in the journal and in saved profiles, so a consistent shape keeps them
    // readable and sortable by category.
    for tweak in &tweaks().tweaks {
        assert!(
            tweak.id.contains('.'),
            "tweak `{}` should be namespaced like `category.thing.action`",
            tweak.id
        );
        assert_eq!(
            tweak.id,
            tweak.id.to_lowercase(),
            "tweak ids must be lowercase"
        );
        assert!(
            tweak
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_'),
            "tweak `{}` has unexpected characters",
            tweak.id
        );
    }
}

#[test]
fn hkcu_only_tweaks_are_marked_as_not_needing_elevation() {
    // Sanity-check the elevation classifier against real data: at least one shipped
    // tweak must be applicable without a UAC prompt, otherwise the unelevated path is
    // dead code and would never be exercised.
    let catalog = tweaks();
    assert!(
        catalog.tweaks.iter().any(|t| !t.requires_elevation()),
        "expected at least one tweak that applies without elevation"
    );
    assert!(
        catalog.tweaks.iter().any(|t| t.requires_elevation()),
        "expected at least one tweak that requires the broker"
    );
}

#[test]
fn windows_11_only_tweaks_are_filtered_out_on_windows_10() {
    let catalog = tweaks();
    let on_win10 = catalog.for_build(19045).count();
    let on_win11 = catalog.for_build(26100).count();

    assert!(
        on_win10 < on_win11,
        "the catalog should contain Windows 11-specific tweaks"
    );
    assert!(on_win10 > 0, "Windows 10 users must still get some tweaks");
}
