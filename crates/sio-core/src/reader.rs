//! Read-only system queries.
//!
//! Separate from [`crate::privileged::PrivilegedOps`] because reading needs no
//! elevation: the app can tell you which tweaks are currently applied without ever
//! showing a UAC prompt. Only *changing* something requires the broker.
//!
//! Keeping this a trait also lets the tweak engine's status logic be tested against a
//! fake machine, which is the only practical way to cover combinations like
//! "half of this tweak is applied".

use crate::error::Result;
use crate::tweak::{Hive, PriorState, PriorValue};
use async_trait::async_trait;

#[async_trait]
pub trait SystemReader: Send + Sync {
    /// Current state of a registry value, including whether it or its key is absent.
    async fn registry_read(&self, hive: Hive, path: &str, name: &str) -> Result<PriorValue>;

    /// A service's start type and whether it is running.
    async fn service_state(&self, name: &str) -> Result<PriorState>;

    /// Whether a UWP package is installed for the current user.
    async fn appx_present(&self, package_family_name: &str) -> Result<bool>;
}
