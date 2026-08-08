//! Package-manager providers for SIO.
//!
//! Three managers behind one trait: winget, Chocolatey and Scoop.
//!
//! The design rests on one observation. None of these tools offer machine-readable
//! output for the operations we need, and all of them print in the user's language —
//! on a Russian Windows, winget answers in Russian. So nothing here parses their text.
//! Decisions come from **exit codes**, which are identical on every Windows language;
//! inventory comes from the structured export formats (`winget export`,
//! `choco list --limit-output`, `scoop export`); and the catalog stores exact package
//! ids so `--exact` skips searching altogether. Raw output is streamed to a log pane
//! for humans and never inspected.
//!
//! Providers build commands and classify results. They never execute anything — the
//! caller decides where a command runs, because winget and Chocolatey need elevation
//! while Scoop must not have it.

#![forbid(unsafe_code)]

pub mod chocolatey;
pub mod installer;
pub mod provider;
pub mod registry;
pub mod scoop;
pub mod winget;

pub use chocolatey::Chocolatey;
pub use installer::{
    install_all, CommandRunner, InstallReport, ItemReport, PlanItem, RoutingRunner,
};
pub use provider::{PackageProvider, Verdict};
pub use registry::ProviderRegistry;
pub use scoop::Scoop;
pub use winget::Winget;
