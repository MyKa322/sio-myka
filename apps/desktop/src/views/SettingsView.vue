<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'

import PageHeader from '@/components/PageHeader.vue'
import StatCard from '@/components/StatCard.vue'
import { LOCALE_NAMES, SUPPORTED_LOCALES, persistLocale, type Locale } from '@/i18n'
import { appVersion, brokerSelfTest, elevationStatus, IpcError } from '@/lib/ipc'
import type { ElevationStatus } from '@/lib/types'
import { useThemeStore } from '@/stores/theme'

const { t, locale } = useI18n()
const theme = useThemeStore()

const elevation = ref<ElevationStatus | null>(null)
const testing = ref(false)
const testResult = ref<{ ok: boolean; message: string; detail?: string } | null>(null)

type UpdateState =
  | { phase: 'idle' }
  | { phase: 'checking' }
  | { phase: 'upToDate' }
  | { phase: 'available'; version: string }
  | { phase: 'downloading' }
  | { phase: 'ready' }
  | { phase: 'failed'; detail: string }

const update = ref<UpdateState>({ phase: 'idle' })
/** Held between the check and the install so we do not fetch the manifest twice. */
let pending: Awaited<ReturnType<typeof import('@tauri-apps/plugin-updater').check>> = null

async function checkUpdates() {
  update.value = { phase: 'checking' }
  try {
    const { check } = await import('@tauri-apps/plugin-updater')
    pending = await check()
    update.value = pending
      ? { phase: 'available', version: pending.version }
      : { phase: 'upToDate' }
  } catch (thrown) {
    // No network, no release published yet, or a dev build with no updater endpoint.
    update.value = { phase: 'failed', detail: String(thrown) }
  }
}

async function installUpdate() {
  if (!pending) return
  update.value = { phase: 'downloading' }
  try {
    await pending.downloadAndInstall()
    update.value = { phase: 'ready' }
  } catch (thrown) {
    update.value = { phase: 'failed', detail: String(thrown) }
  }
}

async function restart() {
  const { relaunch } = await import('@tauri-apps/plugin-process')
  await relaunch()
}

async function refreshElevation() {
  elevation.value = await elevationStatus().catch(() => null)
}

async function runSelfTest() {
  testing.value = true
  testResult.value = null
  try {
    const detail = await brokerSelfTest()
    testResult.value = { ok: true, message: t('settings.adminTestOk'), detail }
  } catch (thrown) {
    const err = thrown as IpcError
    testResult.value = {
      ok: false,
      message: t(err.translationKey ?? 'errors.unknown'),
      detail: err.detail,
    }
  } finally {
    testing.value = false
    await refreshElevation()
  }
}

const THEMES = [
  { value: 'system', key: 'themeSystem' },
  { value: 'light', key: 'themeLight' },
  { value: 'dark', key: 'themeDark' },
] as const

const version = ref('')
onMounted(async () => {
  // A failure here is cosmetic; the rest of Settings must still work.
  version.value = await appVersion().catch(() => '')
  await refreshElevation()
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

      <StatCard :title="t('settings.adminAccess')">
        <p class="text-sm text-pretty text-ink-muted">{{ t('settings.adminHint') }}</p>

        <dl v-if="elevation" class="mt-3 text-sm">
          <div class="flex justify-between gap-4">
            <dt class="text-ink-muted">{{ t('settings.adminState') }}</dt>
            <dd :class="elevation.helperConnected || elevation.alreadyElevated ? 'text-success' : ''">
              {{
                elevation.alreadyElevated
                  ? t('settings.adminAlreadyElevated')
                  : elevation.helperConnected
                    ? t('settings.adminConnected')
                    : t('settings.adminNotConnected')
              }}
            </dd>
          </div>
        </dl>

        <button
          class="mt-4 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink transition-opacity hover:opacity-90 disabled:opacity-50"
          :disabled="testing"
          @click="runSelfTest"
        >
          {{ testing ? t('settings.adminTesting') : t('settings.adminTest') }}
        </button>
        <p class="mt-2 text-xs text-ink-muted">{{ t('settings.adminTestExplain') }}</p>

        <div
          v-if="testResult"
          class="mt-3 rounded-lg border p-3 text-sm"
          :class="
            testResult.ok
              ? 'border-success/40 bg-success/10'
              : 'border-danger/40 bg-danger/10'
          "
          role="status"
        >
          <p class="font-medium">{{ testResult.message }}</p>
          <p v-if="testResult.detail" class="selectable mt-1 font-mono text-xs break-all text-ink-muted">
            {{ testResult.detail }}
          </p>
        </div>
      </StatCard>

      <StatCard :title="t('settings.updates')">
        <p class="text-sm text-pretty text-ink-muted">{{ t('settings.updatesHint') }}</p>

        <div class="mt-3 flex flex-wrap items-center gap-2">
          <button
            class="rounded-lg border border-line px-3 py-1.5 text-sm font-medium transition-colors hover:bg-panel-muted disabled:opacity-50"
            :disabled="update.phase === 'checking' || update.phase === 'downloading'"
            @click="checkUpdates"
          >
            {{ update.phase === 'checking' ? t('settings.checking') : t('settings.checkUpdates') }}
          </button>

          <button
            v-if="update.phase === 'available'"
            class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink transition-opacity hover:opacity-90"
            @click="installUpdate"
          >
            {{ t('settings.installUpdate') }}
          </button>

          <button
            v-if="update.phase === 'ready'"
            class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink transition-opacity hover:opacity-90"
            @click="restart"
          >
            {{ t('settings.restartNow') }}
          </button>
        </div>

        <p v-if="update.phase === 'upToDate'" class="mt-3 text-sm text-success">
          {{ t('settings.upToDate') }}
        </p>
        <p v-else-if="update.phase === 'available'" class="mt-3 text-sm">
          {{ t('settings.updateAvailable', { version: update.version }) }}
        </p>
        <p v-else-if="update.phase === 'downloading'" class="mt-3 text-sm text-ink-muted">
          {{ t('settings.downloading') }}
        </p>
        <p v-else-if="update.phase === 'ready'" class="mt-3 text-sm text-success">
          {{ t('settings.updateReady') }}
        </p>
        <div v-else-if="update.phase === 'failed'" class="mt-3 text-sm">
          <p class="text-danger">{{ t('settings.updateFailed') }}</p>
          <p class="selectable mt-1 font-mono text-xs break-all text-ink-muted">
            {{ update.detail }}
          </p>
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
