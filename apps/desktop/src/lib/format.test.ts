import { describe, expect, it } from 'vitest'
import { formatBytes, formatPercent, usageLevel } from './format'

describe('formatBytes', () => {
  it('uses binary steps so numbers match Windows Explorer', () => {
    // 1024-based: a "1 TB" drive shows as ~931 GB in Explorer, and so must here.
    expect(formatBytes(1024, 'en')).toBe('1.0 KB')
    expect(formatBytes(1024 ** 3, 'en')).toBe('1.0 GB')
    expect(formatBytes(1000 ** 4, 'en')).toBe('931.3 GB')
  })

  it('omits a pointless fraction for raw byte counts', () => {
    expect(formatBytes(512, 'en')).toBe('512 B')
    expect(formatBytes(0, 'en')).toBe('0 B')
  })

  it('formats decimals the way the target locale does', () => {
    // Russian and Ukrainian use a comma as the decimal separator.
    expect(formatBytes(1536, 'ru')).toBe('1,5 KB')
    expect(formatBytes(1536, 'uk')).toBe('1,5 KB')
    expect(formatBytes(1536, 'en')).toBe('1.5 KB')
  })

  it('degrades rather than throwing on nonsense input', () => {
    expect(formatBytes(Number.NaN, 'en')).toBe('—')
    expect(formatBytes(-1, 'en')).toBe('—')
    expect(formatBytes(Number.POSITIVE_INFINITY, 'en')).toBe('—')
  })

  it('caps at the largest known unit instead of running off the end of the array', () => {
    expect(formatBytes(1024 ** 8, 'en')).toContain('PB')
  })
})

describe('formatPercent', () => {
  it('rounds to whole percent', () => {
    expect(formatPercent(75.4, 'en')).toBe('75%')
  })

  it('clamps out-of-range values', () => {
    expect(formatPercent(120, 'en')).toBe('100%')
    expect(formatPercent(-5, 'en')).toBe('0%')
  })

  it('handles non-finite input', () => {
    expect(formatPercent(Number.NaN, 'en')).toBe('—')
  })
})

describe('usageLevel', () => {
  it('bands usage at 75 and 90 percent', () => {
    expect(usageLevel(10)).toBe('ok')
    expect(usageLevel(74.9)).toBe('ok')
    expect(usageLevel(75)).toBe('warn')
    expect(usageLevel(89.9)).toBe('warn')
    expect(usageLevel(90)).toBe('critical')
    expect(usageLevel(100)).toBe('critical')
  })
})
