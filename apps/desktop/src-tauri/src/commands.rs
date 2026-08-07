//! Tauri command handlers.
//!
//! These are adapters and nothing else: deserialize, delegate to a crate, map the
//! result. Any logic that appears here belongs in `crates/` instead, where it can be
//! tested without launching a window.

use crate::error::{CommandError, CommandResult};
use sio_core::sysinfo::SystemSnapshot;

/// Read-only hardware and OS inventory for the dashboard.
#[tauri::command]
pub async fn system_snapshot() -> CommandResult<SystemSnapshot> {
    sio_winsys::probe().await.map_err(CommandError::from)
}

/// The app version, so the UI can show it without duplicating the number.
#[tauri::command]
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
