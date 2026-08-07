//! Tauri command handlers.
//!
//! These are adapters and nothing else: deserialize, delegate to a crate, map the
//! result. Any logic that appears here belongs in `crates/` instead, where it can be
//! tested without launching a window.

use crate::broker_state::BrokerState;
use crate::error::{CommandError, CommandResult};
use serde::Serialize;
use sio_core::sysinfo::SystemSnapshot;
use sio_core::tweak::{Hive, RegistryEdit, RegistryValue};
use tauri::State;

/// Read-only hardware and OS inventory for the dashboard.
#[tauri::command]
pub async fn system_snapshot() -> CommandResult<SystemSnapshot> {
    sio_winsys::probe().await.map_err(CommandError::from)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElevationStatus {
    /// Whether the app itself was started as administrator.
    pub already_elevated: bool,
    /// Whether a privileged route exists, i.e. the next action will *not* prompt.
    pub helper_connected: bool,
}

#[tauri::command]
pub async fn elevation_status(broker: State<'_, BrokerState>) -> CommandResult<ElevationStatus> {
    Ok(ElevationStatus {
        already_elevated: sio_winsys::elevation::is_elevated().unwrap_or(false),
        helper_connected: broker.is_connected().await,
    })
}

/// Prove the elevated path works, end to end.
///
/// Writes a value under `HKLM\SOFTWARE\SIO` — somewhere only an administrator can write
/// — then immediately restores whatever was there before. Exercises the UAC prompt, the
/// pipe handshake, a real privileged write and the revert path, while leaving the
/// registry exactly as it was.
#[tauri::command]
pub async fn broker_self_test(broker: State<'_, BrokerState>) -> CommandResult<String> {
    const PATH: &str = r"SOFTWARE\SIO";
    const NAME: &str = "SelfTest";

    let ops = broker.get().await.map_err(CommandError::from)?;

    let edit = RegistryEdit {
        hive: Hive::Hklm,
        path: PATH.into(),
        name: NAME.into(),
        value: RegistryValue::Dword(1),
    };

    let prior = ops.registry_set(&edit).await.map_err(CommandError::from)?;
    ops.registry_restore(Hive::Hklm, PATH, NAME, &prior)
        .await
        .map_err(CommandError::from)?;

    Ok(format!(
        "wrote and reverted HKLM\\{PATH}\\{NAME}; prior state was {prior:?}"
    ))
}

/// The app version, so the UI can show it without duplicating the number.
#[tauri::command]
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
