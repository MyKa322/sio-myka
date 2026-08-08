import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type {
  ApplyReport,
  AppsResponse,
  CommandErrorPayload,
  ElevationStatus,
  InstallReport,
  JournalEntry,
  Profile,
  Progress,
  ProviderId,
  RevertReport,
  SystemSnapshot,
  TweakView,
} from './types'

/**
 * An error raised by a Rust command.
 *
 * `code` is stable and translatable; `detail` is untranslated English for the log pane.
 * Components should render `t('errors.' + code)` and show `detail` only behind a
 * "Details" affordance.
 */
export class IpcError extends Error {
  readonly code: string
  readonly detail: string

  constructor(payload: CommandErrorPayload) {
    super(payload.detail || payload.code)
    this.name = 'IpcError'
    this.code = payload.code
    this.detail = payload.detail
  }

  /** Translation key for this error. */
  get translationKey(): string {
    return `errors.${this.code}`
  }
}

function isCommandErrorPayload(value: unknown): value is CommandErrorPayload {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as CommandErrorPayload).code === 'string' &&
    typeof (value as CommandErrorPayload).detail === 'string'
  )
}

/**
 * Normalize anything thrown by `invoke` into an `IpcError`.
 *
 * Tauri rejects with the serialized error value, but a panic or a serialization
 * failure rejects with a bare string. Both must end up as the same type or every call
 * site needs its own defensive branch.
 */
export function toIpcError(thrown: unknown): IpcError {
  if (thrown instanceof IpcError) return thrown
  if (isCommandErrorPayload(thrown)) return new IpcError(thrown)
  if (typeof thrown === 'string') return new IpcError({ code: 'unknown', detail: thrown })
  if (thrown instanceof Error) return new IpcError({ code: 'unknown', detail: thrown.message })
  return new IpcError({ code: 'unknown', detail: String(thrown) })
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args)
  } catch (thrown) {
    throw toIpcError(thrown)
  }
}

/** Read-only hardware and OS inventory. */
export function systemSnapshot(): Promise<SystemSnapshot> {
  return call<SystemSnapshot>('system_snapshot')
}

export function appVersion(): Promise<string> {
  return call<string>('app_version')
}

/** Whether we are elevated, and whether the helper is already running. */
export function elevationStatus(): Promise<ElevationStatus> {
  return call<ElevationStatus>('elevation_status')
}

/**
 * Prove the elevated path works end to end.
 *
 * Triggers a UAC prompt on first use. Writes and immediately reverts one value under
 * HKLM, leaving the registry unchanged.
 */
export function brokerSelfTest(): Promise<string> {
  return call<string>('broker_self_test')
}

// --- Apps -------------------------------------------------------------------

/** The catalog, resolved against what this machine can install. */
export function listApps(locale: string): Promise<AppsResponse> {
  return call<AppsResponse>('list_apps', { locale })
}

/** Re-probe the package managers and their inventories. */
export function refreshProviders(): Promise<ProviderId[]> {
  return call<ProviderId[]>('refresh_providers')
}

/**
 * Install a set of catalog apps.
 *
 * Prompts for administrator rights once, up front, then works through the batch.
 * Subscribe with {@link onInstallProgress} before calling to see it happen.
 */
export function installApps(appIds: string[]): Promise<InstallReport> {
  return call<InstallReport>('install_apps', { appIds })
}

/** Subscribe to install progress. Remember to call the returned unlisten function. */
export function onInstallProgress(handler: (progress: Progress) => void): Promise<UnlistenFn> {
  return listen<Progress>('install:progress', (event) => handler(event.payload))
}

// --- Profiles ---------------------------------------------------------------

export function listProfiles(): Promise<Profile[]> {
  return call<Profile[]>('list_profiles')
}

export function saveProfile(name: string, apps: string[], tweaks: string[]): Promise<Profile> {
  return call<Profile>('save_profile', { name, apps, tweaks })
}

export function deleteProfile(name: string): Promise<void> {
  return call<void>('delete_profile', { name })
}

/** Open the profiles folder in Explorer, for copying one to or from a USB stick. */
export function revealProfilesFolder(): Promise<void> {
  return call<void>('reveal_profiles_folder')
}

// --- Tweaks -----------------------------------------------------------------

/**
 * The tweak catalog for this Windows version, each with its current state.
 *
 * Read-only, so this never triggers a UAC prompt — the Tuning screen shows accurate
 * state the moment it opens.
 */
export function listTweaks(locale: string): Promise<TweakView[]> {
  return call<TweakView[]>('list_tweaks', { locale })
}

/** Apply tweaks. Prompts for administrator rights and creates a restore point first. */
export function applyTweaks(tweakIds: string[]): Promise<ApplyReport> {
  return call<ApplyReport>('apply_tweaks', { tweakIds })
}

/** Undo the most recent application of a tweak. */
export function revertTweak(tweakId: string): Promise<RevertReport> {
  return call<RevertReport>('revert_tweak', { tweakId })
}

/** The change history, newest first. */
export function listJournal(): Promise<JournalEntry[]> {
  return call<JournalEntry[]>('list_journal')
}

/** Subscribe to tweak progress. Remember to call the returned unlisten function. */
export function onTweakProgress(handler: (progress: Progress) => void): Promise<UnlistenFn> {
  return listen<Progress>('tweaks:progress', (event) => handler(event.payload))
}
