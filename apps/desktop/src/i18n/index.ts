import { createI18n } from 'vue-i18n'
import en from './locales/en.json'
import ru from './locales/ru.json'
import uk from './locales/uk.json'

export const SUPPORTED_LOCALES = ['en', 'ru', 'uk'] as const
export type Locale = (typeof SUPPORTED_LOCALES)[number]

export const LOCALE_NAMES: Record<Locale, string> = {
  en: 'English',
  ru: 'Русский',
  uk: 'Українська',
}

const STORAGE_KEY = 'sio.locale'

function isSupported(value: string): value is Locale {
  return (SUPPORTED_LOCALES as readonly string[]).includes(value)
}

/**
 * Pick the best locale from an ordered list of preferences.
 *
 * Handles regional tags (`ru-RU` → `ru`) because `navigator.languages` almost always
 * carries them, and falls back to English only once nothing else matches.
 */
export function resolveLocale(preferences: readonly string[]): Locale {
  for (const preference of preferences) {
    if (!preference) continue
    const normalized = preference.toLowerCase()
    if (isSupported(normalized)) return normalized

    const base = normalized.split(/[-_]/)[0]
    if (base && isSupported(base)) return base
  }
  return 'en'
}

/** Saved choice first, then whatever Windows told the webview. */
export function initialLocale(): Locale {
  const saved = localStorage.getItem(STORAGE_KEY)
  if (saved && isSupported(saved)) return saved
  return resolveLocale(navigator.languages ?? [navigator.language])
}

export function persistLocale(locale: Locale): void {
  localStorage.setItem(STORAGE_KEY, locale)
  applyDocumentLocale(locale)
}

/**
 * Keep `<html lang>` in step with the UI language.
 *
 * Assistive technology picks pronunciation from this attribute, so a Russian interface
 * left marked `lang="en"` is read aloud with English phonetics.
 */
export function applyDocumentLocale(locale: Locale): void {
  document.documentElement.lang = locale
}

const startingLocale = initialLocale()
applyDocumentLocale(startingLocale)

export const i18n = createI18n({
  legacy: false,
  locale: startingLocale,
  fallbackLocale: 'en',
  // Our locale files intentionally use `{name}` interpolation only.
  messages: { en, ru, uk },
})
