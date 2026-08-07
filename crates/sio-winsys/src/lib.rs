//! Windows system access.
//!
//! Every platform-specific call and every `unsafe` block in the project lives in this
//! crate. Confining them here means the rest of the workspace stays portable and
//! testable, and any audit of unsafe code has exactly one place to look.

pub mod inventory;
pub mod process;

#[cfg(windows)]
pub mod broker;
#[cfg(windows)]
pub mod elevation;
#[cfg(windows)]
pub mod ops;
#[cfg(windows)]
pub mod registry;
#[cfg(windows)]
pub mod services;
#[cfg(windows)]
pub mod system;

pub use inventory::probe;

#[cfg(windows)]
pub use ops::InProcessOps;
