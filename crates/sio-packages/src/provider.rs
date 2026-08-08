//! The package-provider abstraction.
//!
//! # Why providers do not execute anything
//!
//! A provider *builds* command lines and *classifies* exit codes. It never runs
//! anything itself. That split exists because winget and Chocolatey need administrator
//! rights while Scoop must **not** have them, so the decision of where a command runs
//! belongs to the caller that holds the privileges — not to the provider that knows the
//! package-manager semantics.
//!
//! # Why exit codes and not output
//!
//! None of these tools offer machine-readable output for the operations we need, and
//! their text is localized: on this project's development machine winget answers in
//! Russian. Exit codes are identical on every Windows language, so they are the only
//! thing we branch on. Output is streamed to a log pane for humans and never parsed.

use async_trait::async_trait;
use sio_core::error::{PackageFailure, Result};
use sio_core::package::{InstalledPackage, PackageCmd, PackageRef, ProviderId};

/// How a provider's exit code was interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The desired end state was reached.
    Success,
    /// Nothing to do — already in the desired state.
    AlreadyDone,
    /// Succeeded, but a reboot is needed to finish.
    RebootRequired,
    Failed(PackageFailure),
}

impl Verdict {
    pub fn is_ok(self) -> bool {
        !matches!(self, Verdict::Failed(_))
    }
}

#[async_trait]
pub trait PackageProvider: Send + Sync {
    fn id(&self) -> ProviderId;

    /// Whether this package manager is usable on this machine right now.
    async fn is_available(&self) -> bool;

    /// Everything this manager currently reports as installed.
    async fn installed(&self) -> Result<Vec<InstalledPackage>>;

    /// Build the command that installs a package.
    fn install_cmd(&self, pkg: &PackageRef) -> PackageCmd;

    /// Build the command that removes a package.
    fn uninstall_cmd(&self, pkg: &PackageRef) -> PackageCmd;

    /// Interpret an exit code from one of this provider's commands.
    fn classify(&self, code: i32) -> Verdict;
}

/// Exit codes shared across Windows installer technologies.
///
/// MSI and many EXE installers return these regardless of which package manager
/// invoked them, so every provider checks them before its own table.
pub mod common {
    /// `ERROR_SUCCESS_REBOOT_INITIATED`
    pub const REBOOT_INITIATED: i32 = 1641;
    /// `ERROR_SUCCESS_REBOOT_REQUIRED`
    pub const REBOOT_REQUIRED: i32 = 3010;
}

/// Detect which providers are usable, once.
///
/// Availability is probed by actually running the executable rather than scanning
/// `PATH`, because a shim that exists but cannot run is worse than one that is absent —
/// it produces a confusing failure halfway through a bulk install instead of a clear
/// "not available" up front.
pub async fn detect_available(providers: &[Box<dyn PackageProvider>]) -> Vec<ProviderId> {
    let mut available = Vec::new();
    for provider in providers {
        if provider.is_available().await {
            available.push(provider.id());
        }
    }
    available
}

/// Probe helper: does running `program --version` (or equivalent) succeed?
pub(crate) async fn probe(program: &str, args: &[&str]) -> bool {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    // A spawn failure means the executable is not there. A non-zero exit still proves
    // it exists and runs, which is all we are asking.
    sio_winsys::process::run_captured(program, &args)
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failure_is_not_ok() {
        assert!(Verdict::Success.is_ok());
        assert!(Verdict::AlreadyDone.is_ok());
        assert!(Verdict::RebootRequired.is_ok());
        assert!(!Verdict::Failed(PackageFailure::InstallFailed).is_ok());
    }

    #[tokio::test]
    async fn probe_finds_a_real_program_and_rejects_a_missing_one() {
        assert!(probe("cmd", &["/C", "exit 0"]).await);
        assert!(!probe("sio-definitely-not-installed", &["--version"]).await);
    }

    #[tokio::test]
    async fn probe_treats_a_nonzero_exit_as_present() {
        // "It ran and said no" still proves the tool exists.
        assert!(probe("cmd", &["/C", "exit 1"]).await);
    }
}
