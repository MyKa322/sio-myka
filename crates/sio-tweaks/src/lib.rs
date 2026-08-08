//! The tweak engine.
//!
//! Applies declarative tweaks from the catalog and records a journal that can undo
//! them. The reversible-action model lives in [`sio_core::tweak`]; this crate is the
//! machinery around it.
//!
//! Two properties are worth stating outright, because both are easy to get wrong and
//! neither is visible from the outside until it matters:
//!
//! - **A failed apply is still journalled.** If the third of five actions fails, the
//!   two that succeeded are recorded anyway. Discarding them because the operation
//!   "failed" would leave changes on the machine that nothing knows how to undo.
//! - **Reverting an absent value deletes it.** The journal distinguishes "was set to
//!   zero" from "did not exist", and revert honours the difference. Writing a plausible
//!   default instead is how tuning tools quietly corrupt a system.
//!
//! Reading state needs no elevation, so the UI can show which tweaks are in effect
//! without a UAC prompt; only applying and reverting go through the broker.

#![forbid(unsafe_code)]

pub mod engine;
pub mod journal;

pub use engine::{
    apply, apply_all, revert, status, ApplyReport, RevertReport, TweakOutcome, TweakStatus,
};
pub use journal::JournalStore;
