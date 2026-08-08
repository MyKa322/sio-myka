//! Loading the app and tweak catalogs.
//!
//! A copy of each catalog is compiled into the binary, so a freshly-wiped machine with
//! no network still has a full list — which is exactly the situation this tool exists
//! for. Validation lives in [`sio_core::catalog`] and runs on every load, including the
//! bundled copy: a build that shipped a malformed catalog should fail loudly here
//! rather than render a broken list.

use sio_core::catalog::{AppCatalog, TweakCatalog};
use sio_core::error::Result;

/// Compiled-in copies of the checked-in catalogs.
///
/// CI validates these same files (`crates/sio-core/tests/catalog_files.rs`), so the
/// bundled copy is known-good at build time.
const BUNDLED_APPS: &str = include_str!("../../../../catalog/apps.json");
const BUNDLED_TWEAKS: &str = include_str!("../../../../catalog/tweaks.json");

/// Load the app catalog.
pub fn load_apps() -> Result<AppCatalog> {
    AppCatalog::from_json(BUNDLED_APPS)
}

/// Load the tweak catalog.
pub fn load_tweaks() -> Result<TweakCatalog> {
    TweakCatalog::from_json(BUNDLED_TWEAKS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_app_catalog_loads_and_validates() {
        // If this fails the binary is shipping a catalog it cannot read, which would
        // leave the Apps screen empty on a machine with no network.
        let catalog = load_apps().expect("the bundled catalog must be valid");
        assert!(!catalog.apps.is_empty());
    }

    #[test]
    fn the_bundled_tweak_catalog_loads_and_validates() {
        let catalog = load_tweaks().expect("the bundled tweaks must be valid");
        assert!(!catalog.tweaks.is_empty());
    }

    #[test]
    fn the_bundled_catalog_matches_the_file_on_disk() {
        // Guards against include_str! pointing at a stale or wrong path after a move.
        let on_disk = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../catalog/apps.json"),
        )
        .expect("catalog/apps.json should be readable from the manifest directory");

        assert_eq!(
            on_disk.replace("\r\n", "\n"),
            BUNDLED_APPS.replace("\r\n", "\n"),
            "the compiled-in catalog has drifted from the file"
        );
    }
}
