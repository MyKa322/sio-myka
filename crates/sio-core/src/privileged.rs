//! The single boundary between "things any user can do" and "things needing admin".
//!
//! Everything requiring elevation goes through [`PrivilegedOps`]. There are two
//! implementations: an in-process one that calls Windows directly (used *inside* the
//! elevated broker, and in tests), and a client that forwards over the broker pipe.
//!
//! Keeping this to one trait is what makes the elevation strategy swappable. Falling
//! back to a `requireAdministrator` manifest means constructing the in-process impl at
//! the composition root instead of the broker client — no call sites change.

use crate::error::Result;
use crate::package::PackageCmd;
use crate::progress::ProgressSink;
use crate::tweak::{AppxRef, Hive, PriorState, PriorValue, RegistryEdit, ServiceConfig};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// What actually happened when we asked for a System Restore point.
///
/// Windows silently declines these more often than people expect — it throttles to one
/// per 24 hours and does nothing at all when System Protection is off. Modelling the
/// skip cases explicitly stops the app from claiming a safety net it doesn't have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RestorePointOutcome {
    Created {
        sequence_number: u64,
    },
    /// Windows refused because one was created too recently.
    SkippedThrottled,
    /// System Protection is turned off for the system drive.
    SkippedDisabled,
}

impl RestorePointOutcome {
    /// Whether a usable rollback point now exists.
    pub fn is_protected(&self) -> bool {
        matches!(self, Self::Created { .. })
    }
}

#[async_trait]
pub trait PrivilegedOps: Send + Sync {
    /// Write a registry value, returning what was there before.
    ///
    /// The return value is the entire basis of revert, so implementations must read
    /// before they write, and must distinguish "absent" from "present and zero".
    async fn registry_set(&self, edit: &RegistryEdit) -> Result<PriorValue>;

    /// Put a registry value back to a previously captured state.
    async fn registry_restore(
        &self,
        hive: Hive,
        path: &str,
        name: &str,
        prior: &PriorValue,
    ) -> Result<()>;

    /// Reconfigure a service, returning its previous start type and running state.
    async fn service_configure(&self, cfg: &ServiceConfig) -> Result<PriorState>;

    async fn service_restore(&self, prior: &PriorState) -> Result<()>;

    /// Remove a UWP package. Not reversible by us — see [`crate::tweak::AppliedAction`].
    async fn appx_remove(&self, pkg: &AppxRef) -> Result<()>;

    async fn create_restore_point(&self, description: &str) -> Result<RestorePointOutcome>;

    /// Run a package-manager command, streaming its output.
    ///
    /// Returns the raw exit code. Classifying it is the provider's job, since only the
    /// provider knows what its own codes mean.
    async fn run_package_cmd(&self, cmd: &PackageCmd, progress: ProgressSink) -> Result<i32>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_created_restore_point_counts_as_protection() {
        assert!(RestorePointOutcome::Created { sequence_number: 7 }.is_protected());
        assert!(!RestorePointOutcome::SkippedThrottled.is_protected());
        assert!(!RestorePointOutcome::SkippedDisabled.is_protected());
    }

    #[test]
    fn restore_point_outcome_round_trips() {
        let outcome = RestorePointOutcome::Created {
            sequence_number: 12,
        };
        let json = serde_json::to_string(&outcome).unwrap();
        assert_eq!(
            serde_json::from_str::<RestorePointOutcome>(&json).unwrap(),
            outcome
        );
    }
}
