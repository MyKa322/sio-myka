//! Windows system access.
//!
//! Every platform-specific call and every `unsafe` block in the project lives in this
//! crate. Confining them here means the rest of the workspace stays portable and
//! testable, and any audit of unsafe code has exactly one place to look.

pub mod inventory;

#[cfg(windows)]
pub mod registry;

pub use inventory::probe;
