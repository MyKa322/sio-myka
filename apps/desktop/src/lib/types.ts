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

/** The shape `CommandError` serializes to. */
export interface CommandErrorPayload {
  code: string
  detail: string
}
