import { describe, expect, it } from 'vitest'
import en from './locales/en.json'
import ru from './locales/ru.json'
import uk from './locales/uk.json'
import { resolveLocale, SUPPORTED_LOCALES } from './index'

type Json = Record<string, unknown>

/** Flatten to dotted paths so the comparison reports the exact missing key. */
function keyPaths(obj: Json, prefix = ''): string[] {
  return Object.entries(obj).flatMap(([key, value]) => {
    const path = prefix ? `${prefix}.${key}` : key
    return value && typeof value === 'object' && !Array.isArray(value)
      ? keyPaths(value as Json, path)
      : [path]
  })
}

/** Interpolation placeholders such as `{logical}`. */
function placeholders(value: string): string[] {
  return [...value.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort()
}

function valueAt(obj: Json, path: string): unknown {
  return path.split('.').reduce<unknown>((acc, k) => (acc as Json)?.[k], obj)
}

const locales = { en, ru, uk } as unknown as Record<string, Json>

describe('translation completeness', () => {
  const reference = keyPaths(en as unknown as Json).sort()

  it.each(['ru', 'uk'])('%s defines every key English defines', (name) => {
    const actual = keyPaths(locales[name]!).sort()

    // Reported as explicit lists so a failure names the key instead of a count.
    expect(reference.filter((k) => !actual.includes(k))).toEqual([])
    expect(actual.filter((k) => !reference.includes(k))).toEqual([])
  })

  it.each(['ru', 'uk'])('%s uses the same interpolation placeholders', (name) => {
    const mismatches = reference
      .map((path) => {
        const source = valueAt(en as unknown as Json, path)
        const target = valueAt(locales[name]!, path)
        if (typeof source !== 'string' || typeof target !== 'string') return null
        const a = placeholders(source)
        const b = placeholders(target)
        return a.join(',') === b.join(',') ? null : { path, expected: a, found: b }
      })
      .filter(Boolean)

    // A dropped `{logical}` renders a literal brace to the user, so this must be exact.
    expect(mismatches).toEqual([])
  })

  it.each(['ru', 'uk'])('%s leaves no value untranslated by copy-paste', (name) => {
    // Not a hard rule — some strings are legitimately identical across languages
    // (product names, "SIO"). Only flag long prose that is byte-identical to English.
    const suspicious = reference.filter((path) => {
      const source = valueAt(en as unknown as Json, path)
      const target = valueAt(locales[name]!, path)
      return typeof source === 'string' && source === target && source.length > 24
    })
    expect(suspicious).toEqual([])
  })
})

describe('resolveLocale', () => {
  it('matches an exact tag', () => {
    expect(resolveLocale(['ru'])).toBe('ru')
    expect(resolveLocale(['uk'])).toBe('uk')
  })

  it('strips a region from tags like navigator.languages provides', () => {
    expect(resolveLocale(['ru-RU'])).toBe('ru')
    expect(resolveLocale(['uk-UA'])).toBe('uk')
    expect(resolveLocale(['en-GB'])).toBe('en')
  })

  it('respects preference order and skips unsupported languages', () => {
    expect(resolveLocale(['de-DE', 'pl', 'uk-UA', 'ru'])).toBe('uk')
  })

  it('falls back to English when nothing matches', () => {
    expect(resolveLocale(['de', 'fr'])).toBe('en')
    expect(resolveLocale([])).toBe('en')
  })

  it('ignores empty entries rather than treating them as a match', () => {
    expect(resolveLocale(['', 'ru'])).toBe('ru')
  })

  it('covers every supported locale', () => {
    for (const locale of SUPPORTED_LOCALES) {
      expect(resolveLocale([locale])).toBe(locale)
    }
  })
})
