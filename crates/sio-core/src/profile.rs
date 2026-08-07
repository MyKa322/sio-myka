//! Saved selections — the thing that actually makes a reinstall fast.
//!
//! A profile is a portable list of catalog ids. It stores *references*, never resolved
//! package versions or command lines, so a profile saved a year ago still installs
//! current software today.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub schema_version: u32,
    pub name: String,
    /// Unix milliseconds.
    pub created_at: u64,
    /// [`crate::catalog::AppEntry`] ids.
    #[serde(default)]
    pub apps: Vec<String>,
    /// [`crate::tweak::Tweak`] ids.
    #[serde(default)]
    pub tweaks: Vec<String>,
}

impl Profile {
    pub fn new(name: impl Into<String>, created_at: u64) -> Self {
        Self {
            schema_version: PROFILE_SCHEMA_VERSION,
            name: name.into(),
            created_at,
            apps: Vec::new(),
            tweaks: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty() && self.tweaks.is_empty()
    }

    pub fn from_json(json: &str) -> Result<Self> {
        let profile: Self = serde_json::from_str(json)?;
        if profile.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(Error::Catalog {
                reason: format!(
                    "profile schema_version {} is not supported (expected {PROFILE_SCHEMA_VERSION})",
                    profile.schema_version
                ),
            });
        }
        Ok(profile)
    }

    /// Split the profile's ids against a catalog into resolvable and missing.
    ///
    /// Missing entries are reported rather than silently dropped: a profile carried to
    /// a new machine may reference apps that were since renamed or removed, and the
    /// user needs to know what *didn't* get installed.
    pub fn resolve<'a>(&'a self, known: impl Fn(&str) -> bool) -> Resolution<'a> {
        let (found, missing) = self
            .apps
            .iter()
            .map(String::as_str)
            .partition::<Vec<_>, _>(|id| known(id));
        Resolution { found, missing }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution<'a> {
    pub found: Vec<&'a str>,
    pub missing: Vec<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_profile_is_empty_and_current_version() {
        let p = Profile::new("Gaming PC", 1_700_000_000_000);
        assert!(p.is_empty());
        assert_eq!(p.schema_version, PROFILE_SCHEMA_VERSION);
    }

    #[test]
    fn missing_catalog_entries_are_reported_not_dropped() {
        let mut p = Profile::new("Laptop", 0);
        p.apps = vec!["firefox".into(), "removed-app".into(), "vscode".into()];

        let known = |id: &str| matches!(id, "firefox" | "vscode");
        let res = p.resolve(known);

        assert_eq!(res.found, vec!["firefox", "vscode"]);
        assert_eq!(
            res.missing,
            vec!["removed-app"],
            "the user must be told what was skipped"
        );
    }

    #[test]
    fn round_trips_through_json() {
        let mut p = Profile::new("Work", 42);
        p.apps = vec!["firefox".into()];
        p.tweaks = vec!["privacy.telemetry.disable".into()];

        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(Profile::from_json(&json).unwrap(), p);
    }

    #[test]
    fn unknown_schema_version_is_rejected() {
        let json = r#"{"schema_version":99,"name":"x","created_at":0}"#;
        assert!(Profile::from_json(json).is_err());
    }

    #[test]
    fn absent_app_and_tweak_arrays_default_to_empty() {
        let json = format!(
            r#"{{"schema_version":{PROFILE_SCHEMA_VERSION},"name":"Minimal","created_at":0}}"#
        );
        let p = Profile::from_json(&json).unwrap();
        assert!(p.is_empty());
    }
}
