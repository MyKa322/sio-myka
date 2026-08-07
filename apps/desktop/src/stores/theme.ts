import { defineStore } from 'pinia'
import { computed, ref, watchEffect } from 'vue'

export type ThemePreference = 'system' | 'light' | 'dark'

const STORAGE_KEY = 'sio.theme'
const DARK_QUERY = '(prefers-color-scheme: dark)'

function loadPreference(): ThemePreference {
  const saved = localStorage.getItem(STORAGE_KEY)
  return saved === 'light' || saved === 'dark' || saved === 'system' ? saved : 'system'
}

export const useThemeStore = defineStore('theme', () => {
  const preference = ref<ThemePreference>(loadPreference())

  // Tracked reactively so "Match Windows" follows a theme change made while the app
  // is open, rather than only at startup.
  const media = window.matchMedia(DARK_QUERY)
  const systemPrefersDark = ref(media.matches)
  media.addEventListener('change', (event) => {
    systemPrefersDark.value = event.matches
  })

  const isDark = computed(() =>
    preference.value === 'system' ? systemPrefersDark.value : preference.value === 'dark',
  )

  watchEffect(() => {
    document.documentElement.classList.toggle('dark', isDark.value)
    // Tells the webview to render native widgets (scrollbars, form controls) to match.
    document.documentElement.style.colorScheme = isDark.value ? 'dark' : 'light'
  })

  function setPreference(next: ThemePreference) {
    preference.value = next
    localStorage.setItem(STORAGE_KEY, next)
  }

  return { preference, isDark, setPreference }
})
