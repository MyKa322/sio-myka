/**
 * Locale-aware formatting helpers.
 *
 * These take an explicit locale rather than reading global state so they can be tested
 * directly, and so a screenshot in Russian formats numbers the Russian way.
 */

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'] as const

/**
 * Format a byte count using binary (1024) steps.
 *
 * Storage vendors use 1000-based units, but Windows itself reports 1024-based ones —
 * matching Explorer matters more here than matching the box the drive came in, because
 * the user will compare our number against Explorer's, not the vendor's.
 */
export function formatBytes(bytes: number, locale = 'en', fractionDigits = 1): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—'
  if (bytes === 0) return `0 ${BYTE_UNITS[0]}`

  const exponent = Math.min(Math.floor(Math.log2(bytes) / 10), BYTE_UNITS.length - 1)
  const value = bytes / 1024 ** exponent

  // Whole units read better without a trailing ".0", and byte counts are never
  // fractional.
  const digits = exponent === 0 ? 0 : fractionDigits

  const formatted = new Intl.NumberFormat(locale, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(value)

  return `${formatted} ${BYTE_UNITS[exponent]}`
}

/** Format a 0–100 percentage. */
export function formatPercent(percent: number, locale = 'en'): string {
  if (!Number.isFinite(percent)) return '—'
  return new Intl.NumberFormat(locale, {
    style: 'percent',
    maximumFractionDigits: 0,
  }).format(Math.max(0, Math.min(100, percent)) / 100)
}

/**
 * Severity band for a usage bar, so colour is decided in one place rather than
 * re-derived in every component that draws one.
 */
export function usageLevel(percent: number): 'ok' | 'warn' | 'critical' {
  if (percent >= 90) return 'critical'
  if (percent >= 75) return 'warn'
  return 'ok'
}
