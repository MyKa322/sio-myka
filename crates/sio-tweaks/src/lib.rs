//! The tweak engine.
//!
//! Filled in during M4. Applies declarative tweaks from the catalog and records a
//! journal that can undo them — see [`sio_core::tweak`] for the reversible-action model.

#![forbid(unsafe_code)]
