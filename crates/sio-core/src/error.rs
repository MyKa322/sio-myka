//! Error types shared across every SIO crate.
//!
//! One error enum for the whole domain keeps the `?` operator usable across layer
//! boundaries. Variants carry structured data rather than pre-formatted strings so the
//! UI can localise messages — a formatted English string would be untranslatable.

use crate::package::ProviderId;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("catalog is invalid: {reason}")]
    Catalog { reason: String },

    #[error("package manager `{provider}` is not available on this system")]
    ProviderUnavailable { provider: ProviderId },

    #[error("`{provider}` failed with exit code {code:#010x}")]
    PackageCommand {
        provider: ProviderId,
        code: i32,
        /// Classified meaning of the exit code. This is what the UI branches on;
        /// `code` is kept only for the log pane and bug reports.
        kind: PackageFailure,
    },

    /// The user dismissed the UAC prompt. Distinct from a broker crash: this is a
    /// normal, expected outcome that should never be surfaced as a scary error.
    #[error("the elevation request was declined")]
    ElevationDeclined,

    #[error("elevated helper failed: {reason}")]
    Broker { reason: String },

    #[error("registry operation failed at {path}: {reason}")]
    Registry { path: String, reason: String },

    #[error("windows api call `{api}` failed: {reason}")]
    Windows { api: String, reason: String },

    #[error("tweak `{id}` is not present in the catalog")]
    UnknownTweak { id: String },

    #[error("{0}")]
    Other(String),
}

/// Classification of a package-manager exit code.
///
/// Package managers report *everything* through exit codes, and several non-zero codes
/// are not real failures. Collapsing them all into "error" makes a bulk install of 30
/// apps report a wall of false alarms, so callers branch on this instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageFailure {
    /// Already present at the requested version — a success for our purposes.
    AlreadyInstalled,
    /// No package matched the identifier; usually a stale catalog entry.
    NotFound,
    /// Found, but no installer matches this machine's architecture or OS build.
    NoApplicableInstaller,
    /// The package manager needed elevation it did not have.
    RequiresElevation,
    /// Download or source failure — worth retrying.
    Network,
    /// The install itself ran and failed.
    InstallFailed,
    /// Reboot required before the operation can complete.
    RebootRequired,
    /// Unmapped code. Carries no judgement; shown verbatim with the log.
    Unknown,
}

impl PackageFailure {
    /// Whether the desired end state was reached despite a non-zero exit code.
    ///
    /// Used by bulk operations to decide what counts as a failed item.
    pub fn is_benign(self) -> bool {
        matches!(self, Self::AlreadyInstalled)
    }

    /// Whether retrying the identical command could plausibly succeed.
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::Network)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_installed_is_benign_but_not_retryable() {
        assert!(PackageFailure::AlreadyInstalled.is_benign());
        assert!(!PackageFailure::AlreadyInstalled.is_retryable());
    }

    #[test]
    fn network_failures_are_retryable_but_not_benign() {
        assert!(PackageFailure::Network.is_retryable());
        assert!(!PackageFailure::Network.is_benign());
    }

    #[test]
    fn install_failure_is_neither() {
        assert!(!PackageFailure::InstallFailed.is_benign());
        assert!(!PackageFailure::InstallFailed.is_retryable());
    }
}
