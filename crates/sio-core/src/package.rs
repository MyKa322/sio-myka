//! Package-manager domain types.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Which package manager backs a given package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Winget,
    Chocolatey,
    Scoop,
}

impl ProviderId {
    pub const ALL: [ProviderId; 3] = [Self::Winget, Self::Chocolatey, Self::Scoop];

    /// Name of the executable to probe for on `PATH`.
    pub fn executable(self) -> &'static str {
        match self {
            Self::Winget => "winget",
            Self::Chocolatey => "choco",
            Self::Scoop => "scoop",
        }
    }

    /// Whether this provider needs administrator rights for a machine-scope install.
    ///
    /// Scoop deliberately installs into the user profile and must *not* run elevated —
    /// doing so puts the shims in the wrong profile.
    pub fn requires_elevation(self) -> bool {
        match self {
            Self::Winget | Self::Chocolatey => true,
            Self::Scoop => false,
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.executable())
    }
}

/// A package as addressed by one specific provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageRef {
    pub provider: ProviderId,
    /// Exact provider-native identifier, e.g. `Mozilla.Firefox` for winget.
    ///
    /// Always an exact ID, never a search term — this is what lets us pass `--exact`
    /// and skip parsing the provider's localized search output entirely.
    pub id: String,
    /// Pin to a specific version. `None` means latest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl PackageRef {
    pub fn new(provider: ProviderId, id: impl Into<String>) -> Self {
        Self {
            provider,
            id: id.into(),
            version: None,
        }
    }
}

/// A package discovered as already present on the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub provider: ProviderId,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_version: Option<String>,
}

impl InstalledPackage {
    pub fn has_update(&self) -> bool {
        match &self.available_version {
            Some(v) => v != &self.version,
            None => false,
        }
    }
}

/// The operation to perform on a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageOp {
    Install,
    Uninstall,
    Upgrade,
}

/// A fully-resolved command line, built by a provider and executed by whoever holds
/// the right privileges.
///
/// Providers build these but do not run them. That split is what lets an unelevated
/// provider hand work to the elevated broker without the broker knowing anything about
/// winget or Chocolatey semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageCmd {
    pub provider: ProviderId,
    pub op: PackageOp,
    pub program: String,
    pub args: Vec<String>,
    /// Whether this specific invocation needs to run elevated.
    pub elevated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoop_never_requires_elevation() {
        assert!(!ProviderId::Scoop.requires_elevation());
        assert!(ProviderId::Winget.requires_elevation());
        assert!(ProviderId::Chocolatey.requires_elevation());
    }

    #[test]
    fn update_detection_ignores_missing_available_version() {
        let mut pkg = InstalledPackage {
            provider: ProviderId::Winget,
            id: "Mozilla.Firefox".into(),
            name: "Firefox".into(),
            version: "120.0".into(),
            available_version: None,
        };
        assert!(!pkg.has_update());

        pkg.available_version = Some("120.0".into());
        assert!(!pkg.has_update(), "same version is not an update");

        pkg.available_version = Some("121.0".into());
        assert!(pkg.has_update());
    }

    #[test]
    fn provider_id_serialises_as_lowercase() {
        let json = serde_json::to_string(&ProviderId::Chocolatey).unwrap();
        assert_eq!(json, "\"chocolatey\"");
    }

    #[test]
    fn package_ref_omits_absent_version() {
        let json = serde_json::to_string(&PackageRef::new(ProviderId::Winget, "Foo.Bar")).unwrap();
        assert!(!json.contains("version"), "got {json}");
    }
}
