<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'

import PageHeader from '@/components/PageHeader.vue'
import StatCard from '@/components/StatCard.vue'
import { LOCALE_NAMES, SUPPORTED_LOCALES, persistLocale, type Locale } from '@/i18n'
import { appVersion } from '@/lib/ipc'
import { useThemeStore } from '@/stores/theme'

const { t, locale } = useI18n()
const theme = useThemeStore()

const THEMES = [
  { value: 'system', key: 'themeSystem' },
  { value: 'light', key: 'themeLight' },
  { value: 'dark', key: 'themeDark' },
] as const

const version = ref('')
onMounted(async () => {
  // A failure here is cosmetic; the rest of Settings must still work.
  version.value = await appVersion().catch(() => '')
})

function changeLocale(next: Locale) {
  locale.value = next
  persistLocale(next)
}
</script>

<template>
  <div class="p-6 lg:p-8">
    <PageHeader :title="t('settings.title')" :subtitle="t('settings.subtitle')" />

    <div class="grid max-w-3xl gap-4">
      <StatCard :title="t('settings.language')">
        <div class="flex flex-wrap gap-2">
          <button
            v-for="code in SUPPORTED_LOCALES"
            :key="code"
            class="rounded-lg border px-3 py-1.5 text-sm font-medium transition-colors"
            :class="
              locale === code
                ? 'border-accent bg-accent text-accent-ink'
                : 'border-line text-ink-muted hover:bg-panel-muted hover:text-ink'
            "
            :aria-pressed="locale === code"
            @click="changeLocale(code)"
          >
            {{ LOCALE_NAMES[code] }}
          </button>
        </div>
        <p class="mt-3 text-sm text-ink-muted">{{ t('settings.languageHint') }}</p>
      </StatCard>

      <StatCard :title="t('settings.theme')">
        <div class="flex flex-wrap gap-2">
          <button
            v-for="option in THEMES"
            :key="option.value"
            class="rounded-lg border px-3 py-1.5 text-sm font-medium transition-colors"
            :class="
              theme.preference === option.value
                ? 'border-accent bg-accent text-accent-ink'
                : 'border-line text-ink-muted hover:bg-panel-muted hover:text-ink'
            "
            :aria-pressed="theme.preference === option.value"
            @click="theme.setPreference(option.value)"
          >
            {{ t(`settings.${option.key}`) }}
          </button>
        </div>
      </StatCard>

      <StatCard :title="t('settings.about')">
        <dl class="text-sm">
          <div class="flex justify-between gap-4">
            <dt class="text-ink-muted">{{ t('settings.version') }}</dt>
            <dd class="selectable font-mono tabular-nums">{{ version || '—' }}</dd>
          </div>
        </dl>
      </StatCard>
    </div>
  </div>
</template>
