//! Package-manager providers.
//!
//! Filled in during M3. The trait lives here; the winget, Chocolatey and Scoop
//! implementations are driven by exit codes rather than parsed output, because
//! package-manager stdout is localized and cannot be relied on.

#![forbid(unsafe_code)]
