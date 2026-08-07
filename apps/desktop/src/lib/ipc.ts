import { invoke } from '@tauri-apps/api/core'
import type { CommandErrorPayload, SystemSnapshot } from './types'

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
