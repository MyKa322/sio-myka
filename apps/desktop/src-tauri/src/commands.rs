//! Tauri command handlers.
//!
//! These are adapters and nothing else: deserialize, delegate to a crate, map the
//! result. Any logic that appears here belongs in `crates/` instead, where it can be
//! tested without launching a window.

use crate::broker_state::BrokerState;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;
use serde::Serialize;
use sio_core::package::ProviderId;
use sio_core::profile::Profile;
use sio_core::progress::ProgressSink;
use sio_core::sysinfo::SystemSnapshot;
use sio_core::tweak::{Hive, RegistryEdit, RegistryValue};
use sio_packages::installer::{self, PlanItem, RoutingRunner};
use sio_tweaks::TweakStatus;
use tauri::{AppHandle, Emitter, State};

/// Event name for streamed install progress.
pub const INSTALL_PROGRESS_EVENT: &str = "install:progress";

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

// ---------------------------------------------------------------------------
// Elevation
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Apps
// ---------------------------------------------------------------------------

/// One catalog app, flattened and localized for display.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub homepage: Option<String>,
    pub tags: Vec<String>,
    /// Whether some available provider can install it.
    pub installable: bool,
    /// Whether it already appears in a provider's inventory.
    pub installed: bool,
    /// The provider that would be used.
    pub provider: Option<ProviderId>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppsResponse {
    pub apps: Vec<AppView>,
    pub available_providers: Vec<ProviderId>,
}

/// The catalog, resolved against what this machine can actually install.
#[tauri::command]
pub async fn list_apps(state: State<'_, AppState>, locale: String) -> CommandResult<AppsResponse> {
    let providers = state.providers().await;
    let available = providers.available().to_vec();

    let apps = state
        .apps()
        .apps
        .iter()
        .map(|entry| {
            let source = entry.preferred_source(&available);
            AppView {
                id: entry.id.clone(),
                name: entry.name.clone(),
                description: entry.description.get(&locale).to_string(),
                category: format!("{:?}", entry.category).to_lowercase(),
                homepage: entry.homepage.clone(),
                tags: entry.tags.clone(),
                installable: source.is_some(),
                installed: entry
                    .sources
                    .iter()
                    .any(|s| providers.is_installed(s.provider, &s.id)),
                provider: source.map(|s| s.provider),
            }
        })
        .collect();

    Ok(AppsResponse {
        apps,
        available_providers: available,
    })
}

/// Re-probe the package managers and their inventories.
#[tauri::command]
pub async fn refresh_providers(state: State<'_, AppState>) -> CommandResult<Vec<ProviderId>> {
    Ok(state.refresh_providers().await.available().to_vec())
}

/// Install a set of catalog apps, streaming progress as `install:progress` events.
#[tauri::command]
pub async fn install_apps(
    app: AppHandle,
    state: State<'_, AppState>,
    broker: State<'_, BrokerState>,
    app_ids: Vec<String>,
) -> CommandResult<installer::InstallReport> {
    let providers = state.providers().await;
    let available = providers.available().to_vec();

    // Resolve ids to concrete packages first. An id with no usable source is dropped
    // here and reported by the installer, rather than failing the whole batch.
    let items: Vec<PlanItem> = app_ids
        .iter()
        .filter_map(|id| {
            let entry = state.apps().get(id)?;
            let source = entry.preferred_source(&available)?;
            Some(PlanItem {
                app_id: entry.id.clone(),
                display_name: entry.name.clone(),
                package: source.clone(),
            })
        })
        .collect();

    // Elevation is requested once, up front, so the UAC prompt appears before the
    // batch starts rather than in the middle of it.
    let ops = broker.get().await.map_err(CommandError::from)?;
    let runner = RoutingRunner::new(ops);

    let (sink, mut rx) = ProgressSink::new();
    let emitter = app.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            // A failed emit means the window went away; the install continues.
            let _ = emitter.emit(INSTALL_PROGRESS_EVENT, &progress);
        }
    });

    let report = installer::install_all(&items, &providers, &runner, sink).await;
    let _ = forwarder.await;

    // Inventory is stale now that things were installed.
    state.refresh_providers().await;

    Ok(report)
}

// ---------------------------------------------------------------------------
// Profiles
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_profiles() -> CommandResult<Vec<Profile>> {
    crate::profiles::list().await.map_err(CommandError::from)
}

#[tauri::command]
pub async fn save_profile(
    name: String,
    apps: Vec<String>,
    tweaks: Vec<String>,
) -> CommandResult<Profile> {
    let mut profile = Profile::new(name, sio_core::now_unix_ms());
    profile.apps = apps;
    profile.tweaks = tweaks;

    crate::profiles::save(&profile)
        .await
        .map_err(CommandError::from)?;
    Ok(profile)
}

#[tauri::command]
pub async fn delete_profile(name: String) -> CommandResult<()> {
    crate::profiles::delete(&name)
        .await
        .map_err(CommandError::from)
}

/// Open the profiles folder, so a profile can be copied to or from a USB stick.
#[tauri::command]
pub async fn reveal_profiles_folder() -> CommandResult<()> {
    crate::profiles::reveal_folder()
        .await
        .map_err(CommandError::from)
}

// ---------------------------------------------------------------------------
// Tweaks
// ---------------------------------------------------------------------------

/// Event name for streamed tweak progress.
pub const TWEAK_PROGRESS_EVENT: &str = "tweaks:progress";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TweakView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub risk: String,
    pub requires_restart: bool,
    pub requires_elevation: bool,
    pub status: TweakStatus,
    /// True when undoing it cannot fully restore the machine — Appx removal.
    pub irreversible: bool,
}

/// The tweak catalog for this Windows version, with each tweak's current state.
///
/// Reads only, so this never triggers a UAC prompt.
#[tauri::command]
pub async fn list_tweaks(
    state: State<'_, AppState>,
    locale: String,
) -> CommandResult<Vec<TweakView>> {
    let reader = state.reader();
    let mut out = Vec::new();

    for tweak in state.applicable_tweaks() {
        out.push(TweakView {
            id: tweak.id.clone(),
            name: tweak.name.get(&locale).to_string(),
            description: tweak.description.get(&locale).to_string(),
            category: format!("{:?}", tweak.category).to_lowercase(),
            risk: format!("{:?}", tweak.risk).to_lowercase(),
            requires_restart: tweak.requires_restart,
            requires_elevation: tweak.requires_elevation(),
            status: sio_tweaks::status(reader, tweak).await,
            irreversible: tweak
                .actions
                .iter()
                .any(|a| matches!(a, sio_core::tweak::TweakAction::Appx(_))),
        });
    }

    Ok(out)
}

/// Apply tweaks, streaming progress as `tweaks:progress` events.
#[tauri::command]
pub async fn apply_tweaks(
    app: AppHandle,
    state: State<'_, AppState>,
    broker: State<'_, BrokerState>,
    tweak_ids: Vec<String>,
) -> CommandResult<sio_tweaks::ApplyReport> {
    let selected: Vec<_> = state
        .applicable_tweaks()
        .filter(|t| tweak_ids.contains(&t.id))
        .cloned()
        .collect();

    let ops = broker.get().await.map_err(CommandError::from)?;

    let (sink, mut rx) = ProgressSink::new();
    let emitter = app.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            let _ = emitter.emit(TWEAK_PROGRESS_EVENT, &progress);
        }
    });

    let report = sio_tweaks::apply_all(
        ops.as_ref(),
        state.journal(),
        &selected,
        sio_core::now_unix_ms(),
        sink,
    )
    .await;
    let _ = forwarder.await;

    // Removing packages changes what a status check would report.
    state.reader().invalidate().await;

    Ok(report)
}

/// Undo the most recent application of a tweak.
#[tauri::command]
pub async fn revert_tweak(
    state: State<'_, AppState>,
    broker: State<'_, BrokerState>,
    tweak_id: String,
) -> CommandResult<sio_tweaks::RevertReport> {
    let entry = state
        .journal()
        .newest_active_for(&tweak_id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| {
            CommandError::from(sio_core::Error::UnknownTweak {
                id: tweak_id.clone(),
            })
        })?;

    let ops = broker.get().await.map_err(CommandError::from)?;
    let report = sio_tweaks::revert(
        ops.as_ref(),
        state.journal(),
        &entry,
        sio_core::now_unix_ms(),
    )
    .await
    .map_err(CommandError::from)?;

    state.reader().invalidate().await;
    Ok(report)
}

/// The change history, newest first.
#[tauri::command]
pub async fn list_journal(
    state: State<'_, AppState>,
) -> CommandResult<Vec<sio_core::tweak::JournalEntry>> {
    state.journal().list().await.map_err(CommandError::from)
}
