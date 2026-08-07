import { describe, expect, it } from 'vitest'
import { IpcError, toIpcError } from './ipc'

describe('toIpcError', () => {
  it('preserves a structured error from Rust', () => {
    const err = toIpcError({ code: 'elevationDeclined', detail: 'the user said no' })
    expect(err).toBeInstanceOf(IpcError)
    expect(err.code).toBe('elevationDeclined')
    expect(err.detail).toBe('the user said no')
  })

  it('wraps a bare string, which is what a Rust panic rejects with', () => {
    const err = toIpcError('called `Option::unwrap()` on a `None` value')
    expect(err.code).toBe('unknown')
    expect(err.detail).toContain('unwrap')
  })

  it('wraps a JS Error thrown before the call reached Rust', () => {
    expect(toIpcError(new Error('offline')).detail).toBe('offline')
  })

  it('does not double-wrap', () => {
    const original = new IpcError({ code: 'registryFailed', detail: 'access denied' })
    expect(toIpcError(original)).toBe(original)
  })

  it('survives values that are neither strings nor objects', () => {
    expect(toIpcError(undefined).code).toBe('unknown')
    expect(toIpcError(null).code).toBe('unknown')
    expect(toIpcError(42).detail).toBe('42')
  })

  it('rejects a partial payload rather than producing an undefined code', () => {
    // A malformed payload must not yield `code: undefined`, which would render the
    // literal string "errors.undefined" to the user.
    const err = toIpcError({ code: 'oops' })
    expect(err.code).toBe('unknown')
  })

  it('exposes a translation key matching the locale files', () => {
    const err = new IpcError({ code: 'providerUnavailable', detail: '' })
    expect(err.translationKey).toBe('errors.providerUnavailable')
  })
})
