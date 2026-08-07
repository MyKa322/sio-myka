//! The app and tweak catalogs.
//!
//! Both are plain JSON data files under `catalog/`. A copy is compiled into the binary
//! so a freshly-reinstalled machine with no network still works, and a newer copy is
//! fetched from the repository at runtime. Adding an app is therefore a commit, not a
//! release.

use crate::error::{Error, Result};
use crate::package::{PackageRef, ProviderId};
use crate::text::LocalizedText;
use crate::tweak::Tweak;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Bumped only on a breaking schema change. An app refuses to load a catalog whose
/// major version it does not understand, and falls back to its bundled copy.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppCategory {
    Browser,
    Communication,
    Development,
    Media,
    Graphics,
    Gaming,
    Utilities,
    Security,
    Office,
    Drivers,
}

/// One catalog app, which may be obtainable from several package managers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEntry {
    /// Stable slug, e.g. `firefox`. Referenced by profiles, so never rename it.
    pub id: String,
    /// Product name. Not localized — brand names don't translate.
    pub name: String,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub description: LocalizedText,
    pub category: AppCategory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Where this app can be obtained, in order of preference.
    pub sources: Vec<PackageRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl AppEntry {
    /// Pick the best source given which package managers are actually usable.
    ///
    /// Preference follows catalog order, so a curator can put the source that produces
    /// the better install (correct architecture, no bundled extras) first.
    pub fn preferred_source(&self, available: &[ProviderId]) -> Option<&PackageRef> {
        self.sources
            .iter()
            .find(|s| available.contains(&s.provider))
    }

    /// Whether this app can be installed at all right now.
    pub fn is_installable(&self, available: &[ProviderId]) -> bool {
        self.preferred_source(available).is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppCatalog {
    pub schema_version: u32,
    pub apps: Vec<AppEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TweakCatalog {
    pub schema_version: u32,
    pub tweaks: Vec<Tweak>,
}

/// Shared validation rules. Run on load *and* in CI against the checked-in files, so a
/// malformed catalog is caught at review time rather than on a user's machine.
pub trait Validate {
    fn validate(&self) -> Result<()>;
}

fn check_schema_version(found: u32) -> Result<()> {
    if found != SCHEMA_VERSION {
        return Err(Error::Catalog {
            reason: format!("unsupported schema_version {found}, expected {SCHEMA_VERSION}"),
        });
    }
    Ok(())
}

/// Reject duplicate identifiers.
///
/// Duplicates are worse than they look: profiles reference entries by id, so two
/// entries sharing one id makes a saved profile silently ambiguous.
fn check_unique_ids<'a>(ids: impl Iterator<Item = &'a str>, what: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(Error::Catalog {
                reason: format!("duplicate {what} id `{id}`"),
            });
        }
    }
    Ok(())
}

impl Validate for AppCatalog {
    fn validate(&self) -> Result<()> {
        check_schema_version(self.schema_version)?;
        check_unique_ids(self.apps.iter().map(|a| a.id.as_str()), "app")?;

        for app in &self.apps {
            if app.sources.is_empty() {
                return Err(Error::Catalog {
                    reason: format!("app `{}` has no sources and can never be installed", app.id),
                });
            }
            if app.id.trim().is_empty() {
                return Err(Error::Catalog {
                    reason: "an app has an empty id".into(),
                });
            }
        }
        Ok(())
    }
}

impl Validate for TweakCatalog {
    fn validate(&self) -> Result<()> {
        check_schema_version(self.schema_version)?;
        check_unique_ids(self.tweaks.iter().map(|t| t.id.as_str()), "tweak")?;

        for tweak in &self.tweaks {
            if tweak.actions.is_empty() {
                return Err(Error::Catalog {
                    reason: format!("tweak `{}` has no actions and would do nothing", tweak.id),
                });
            }
        }
        Ok(())
    }
}

impl AppCatalog {
    pub fn get(&self, id: &str) -> Option<&AppEntry> {
        self.apps.iter().find(|a| a.id == id)
    }

    /// Parse and validate in one step, so an invalid catalog can never be constructed
    /// from untrusted input.
    pub fn from_json(json: &str) -> Result<Self> {
        let catalog: Self = serde_json::from_str(json)?;
        catalog.validate()?;
        Ok(catalog)
    }
}

impl TweakCatalog {
    pub fn get(&self, id: &str) -> Option<&Tweak> {
        self.tweaks.iter().find(|t| t.id == id)
    }

    pub fn from_json(json: &str) -> Result<Self> {
        let catalog: Self = serde_json::from_str(json)?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// Tweaks applicable to a given Windows build.
    pub fn for_build(&self, build: u32) -> impl Iterator<Item = &Tweak> {
        self.tweaks.iter().filter(move |t| t.applies.matches(build))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, sources: Vec<PackageRef>) -> AppEntry {
        AppEntry {
            id: id.into(),
            name: id.into(),
            description: LocalizedText::new(),
            category: AppCategory::Utilities,
            homepage: None,
            sources,
            tags: vec![],
        }
    }

    #[test]
    fn preferred_source_follows_catalog_order_not_provider_order() {
        let app = entry(
            "vscode",
            vec![
                PackageRef::new(ProviderId::Winget, "Microsoft.VisualStudioCode"),
                PackageRef::new(ProviderId::Chocolatey, "vscode"),
            ],
        );

        // Both available: the curator's first choice wins.
        let both = [ProviderId::Chocolatey, ProviderId::Winget];
        assert_eq!(
            app.preferred_source(&both).unwrap().provider,
            ProviderId::Winget
        );

        // Winget missing: fall through to the next listed source.
        let choco_only = [ProviderId::Chocolatey];
        assert_eq!(
            app.preferred_source(&choco_only).unwrap().provider,
            ProviderId::Chocolatey
        );
    }

    #[test]
    fn app_with_no_usable_provider_is_not_installable() {
        let app = entry(
            "scoop-only",
            vec![PackageRef::new(ProviderId::Scoop, "thing")],
        );
        assert!(!app.is_installable(&[ProviderId::Winget]));
        assert!(app.is_installable(&[ProviderId::Scoop]));
    }

    #[test]
    fn duplicate_app_ids_are_rejected() {
        let catalog = AppCatalog {
            schema_version: SCHEMA_VERSION,
            apps: vec![
                entry(
                    "firefox",
                    vec![PackageRef::new(ProviderId::Winget, "Mozilla.Firefox")],
                ),
                entry(
                    "firefox",
                    vec![PackageRef::new(ProviderId::Chocolatey, "firefox")],
                ),
            ],
        };
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("duplicate"), "got: {err}");
    }

    #[test]
    fn app_without_sources_is_rejected() {
        let catalog = AppCatalog {
            schema_version: SCHEMA_VERSION,
            apps: vec![entry("ghost", vec![])],
        };
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("no sources"), "got: {err}");
    }

    #[test]
    fn future_schema_version_is_rejected_so_we_fall_back_to_the_bundled_copy() {
        let catalog = AppCatalog {
            schema_version: SCHEMA_VERSION + 1,
            apps: vec![],
        };
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("unsupported schema_version"), "got: {err}");
    }

    #[test]
    fn from_json_validates_rather_than_just_deserialising() {
        let json = format!(
            r#"{{"schema_version":{SCHEMA_VERSION},"apps":[
                {{"id":"a","name":"A","category":"utilities","sources":[]}}
            ]}}"#
        );
        assert!(
            AppCatalog::from_json(&json).is_err(),
            "empty sources must not survive parsing"
        );
    }

    #[test]
    fn valid_catalog_round_trips() {
        let catalog = AppCatalog {
            schema_version: SCHEMA_VERSION,
            apps: vec![entry(
                "firefox",
                vec![PackageRef::new(ProviderId::Winget, "Mozilla.Firefox")],
            )],
        };
        catalog.validate().unwrap();

        let json = serde_json::to_string(&catalog).unwrap();
        assert_eq!(AppCatalog::from_json(&json).unwrap(), catalog);
        assert_eq!(catalog.get("firefox").unwrap().name, "firefox");
        assert!(catalog.get("nope").is_none());
    }
}
