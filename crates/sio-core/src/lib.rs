//! Domain model for SIO — a post-reinstall setup and tuning tool for Windows.
//!
//! This crate is deliberately platform-agnostic and dependency-light: it contains the
//! types, invariants and protocol definitions, and no I/O. Everything here compiles and
//! tests on any platform, which is what keeps the interesting logic — revert planning,
//! catalog validation, exit-code classification — cheap to test.
//!
//! # Layout
//!
//! - [`catalog`] — the app and tweak catalogs, and their validation rules
//! - [`error`] — the shared error type and package-failure classification
//! - [`package`] — package-manager identifiers and resolved commands
//! - [`privileged`] — the single trait separating elevated from unelevated work
//! - [`profile`] — saved selections, the payload you carry to a fresh install
//! - [`progress`] — progress reporting for long-running operations
//! - [`protocol`] — the UI ↔ elevated-broker wire format
//! - [`sysinfo`] — the read-only system inventory shown on the dashboard
//! - [`text`] — localized strings carried inside the catalog
//! - [`tweak`] — tweak definitions and the reversible-action model

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod catalog;
pub mod error;
pub mod package;
pub mod privileged;
pub mod profile;
pub mod progress;
pub mod protocol;
pub mod sysinfo;
pub mod text;
pub mod tweak;

pub use error::{Error, PackageFailure, Result};

/// Re-exports of the types most call sites need.
pub mod prelude {
    pub use crate::catalog::{AppCatalog, AppEntry, TweakCatalog, Validate};
    pub use crate::error::{Error, PackageFailure, Result};
    pub use crate::package::{InstalledPackage, PackageCmd, PackageOp, PackageRef, ProviderId};
    pub use crate::privileged::{PrivilegedOps, RestorePointOutcome};
    pub use crate::profile::Profile;
    pub use crate::progress::{Outcome, Progress, ProgressSink};
    pub use crate::sysinfo::SystemSnapshot;
    pub use crate::text::LocalizedText;
    pub use crate::tweak::{AppliedAction, JournalEntry, Tweak, TweakAction};
}

/// Current wall-clock time in Unix milliseconds.
///
/// Used for journal and profile timestamps. Saturates at zero rather than panicking if
/// the system clock is set before 1970 — a wrong timestamp on a journal entry is a
/// cosmetic problem; a panic during a registry write is not.
pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_after_2020() {
        // 2020-01-01T00:00:00Z. Guards against a units mix-up (seconds vs millis).
        assert!(now_unix_ms() > 1_577_836_800_000);
    }
}
