/**
 * Mirrors of the Rust types crossing the IPC boundary.
 *
 * Field names are snake_case because that is what serde emits by default and the Rust
 * side is the source of truth. Renaming on one side only would produce silent
 * `undefined`s at runtime, which TypeScript cannot catch across `invoke`.
 */

export type ActivationStatus =
  | 'licensed'
  | 'grace_period'
  | 'notification'
  | 'unlicensed'
  | 'unknown'

export type DiskKind = 'ssd' | 'hdd' | 'unknown'

export interface OsInfo {
  edition: string
  display_version: string
  build: number
  arch: string
  machine_name: string
  activation: ActivationStatus
}

export interface CpuInfo {
  brand: string
  physical_cores: number | null
  logical_cores: number
}

export interface MemoryInfo {
  total_bytes: number
  used_bytes: number
}

export interface GpuInfo {
  name: string
  vram_bytes?: number
  driver_version?: string
}

export interface DiskInfo {
  mount_point: string
  total_bytes: number
  available_bytes: number
  is_removable: boolean
  kind: DiskKind
}

export interface SystemSnapshot {
  os: OsInfo
  cpu: CpuInfo
  memory: MemoryInfo
  gpus: GpuInfo[]
  disks: DiskInfo[]
}

export interface ElevationStatus {
  alreadyElevated: boolean
  helperConnected: boolean
}

export type ProviderId = 'winget' | 'chocolatey' | 'scoop'

export interface AppView {
  id: string
  name: string
  description: string
  category: string
  homepage: string | null
  tags: string[]
  installable: boolean
  installed: boolean
  provider: ProviderId | null
}

export interface AppsResponse {
  apps: AppView[]
  availableProviders: ProviderId[]
}

/**
 * `Outcome` flattened into `ItemReport`, so `status` sits alongside the other fields.
 * The extra key depends on the variant: `reason` for skipped, `message` for failed.
 */
export type ItemReport = {
  appId: string
  displayName: string
  exitCode?: number
  rebootRequired: boolean
} & (
  | { status: 'success' }
  | { status: 'skipped'; reason: string }
  | { status: 'failed'; message: string }
)

export interface InstallReport {
  items: ItemReport[]
}

/** Payload of the `install:progress` event. */
export type Progress =
  | { kind: 'started'; item: string }
  | { kind: 'log'; line: string }
  | { kind: 'percent'; item: string; percent: number }
  | {
      kind: 'finished'
      item: string
      outcome:
        | { status: 'success' }
        | { status: 'skipped'; reason: string }
        | { status: 'failed'; message: string }
    }

export interface Profile {
  schema_version: number
  name: string
  created_at: number
  apps: string[]
  tweaks: string[]
}

/** The shape `CommandError` serializes to. */
export interface CommandErrorPayload {
  code: string
  detail: string
}
