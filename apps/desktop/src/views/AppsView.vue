<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery } from '@tanstack/vue-query'
import type { UnlistenFn } from '@tauri-apps/api/event'

import AppIcon from '@/components/AppIcon.vue'
import PageHeader from '@/components/PageHeader.vue'
import { installApps, listApps, onInstallProgress, type IpcError } from '@/lib/ipc'
import type { InstallReport, ItemReport } from '@/lib/types'
import { useSelectionStore } from '@/stores/selection'

const { t, locale } = useI18n()
const selection = useSelectionStore()

const { data, isPending, error, refetch } = useQuery({
  queryKey: computed(() => ['apps', locale.value]),
  queryFn: () => listApps(locale.value),
})

const search = ref('')
const category = ref<string | null>(null)

const installing = ref(false)
const logLines = ref<string[]>([])
const currentItem = ref('')
const report = ref<InstallReport | null>(null)
const installError = ref<string | null>(null)
let unlisten: UnlistenFn | null = null

const categories = computed(() => {
  const seen = new Set((data.value?.apps ?? []).map((a) => a.category))
  return [...seen].sort()
})

const visible = computed(() => {
  const needle = search.value.trim().toLowerCase()
  return (data.value?.apps ?? []).filter((app) => {
    if (category.value && app.category !== category.value) return false
    if (!needle) return true
    return (
      app.name.toLowerCase().includes(needle) ||
      app.description.toLowerCase().includes(needle) ||
      app.tags.some((tag) => tag.includes(needle))
    )
  })
})

/** Only apps that can actually be installed are worth selecting. */
const selectable = computed(() => visible.value.filter((a) => a.installable && !a.installed))

function selectAllVisible() {
  selection.addApps(selectable.value.map((a) => a.id))
}

function statusOf(item: ItemReport): string {
  return t(`status.${item.status}`)
}

function detailOf(item: ItemReport): string | null {
  if (item.status === 'failed') return item.message
  if (item.status === 'skipped') return item.reason
  return null
}

async function runInstall() {
  installing.value = true
  report.value = null
  installError.value = null
  logLines.value = []
  currentItem.value = ''

  unlisten = await onInstallProgress((progress) => {
    switch (progress.kind) {
      case 'started':
        currentItem.value = progress.item
        break
      case 'log':
        // Cap the buffer: a big batch produces thousands of lines and the DOM does
        // not need to hold all of them.
        logLines.value = [...logLines.value.slice(-400), progress.line]
        break
      case 'finished':
        currentItem.value = ''
        break
    }
  })

  try {
    report.value = await installApps(selection.appIds)
    selection.clear()
    await refetch()
  } catch (thrown) {
    const err = thrown as IpcError
    installError.value = t(err.translationKey ?? 'errors.unknown')
  } finally {
    unlisten?.()
    unlisten = null
    installing.value = false
  }
}

onUnmounted(() => unlisten?.())
</script>

<template>
  <div class="flex h-full flex-col">
    <div class="shrink-0 p-6 pb-0 lg:p-8 lg:pb-0">
      <PageHeader :title="t('apps.title')" :subtitle="t('apps.subtitle')" />

      <p v-if="isPending" class="text-sm text-ink-muted">{{ t('common.loading') }}</p>

      <div
        v-else-if="error"
        class="rounded-panel border border-danger/40 bg-danger/10 p-4"
        role="alert"
      >
        <p class="text-sm font-medium">{{ t((error as IpcError).translationKey) }}</p>
        <p class="selectable mt-1 font-mono text-xs break-all text-ink-muted">
          {{ (error as IpcError).detail }}
        </p>
      </div>

      <template v-else-if="data">
        <div
          v-if="!data.availableProviders.length"
          class="mb-4 rounded-panel border border-warning/40 bg-warning/10 p-4 text-sm"
          role="alert"
        >
          {{ t('apps.noSources') }}
        </div>

        <div class="mb-4 flex flex-wrap items-center gap-2">
          <input
            v-model="search"
            type="search"
            :placeholder="t('apps.searchPlaceholder')"
            class="selectable min-w-48 flex-1 rounded-lg border border-line bg-panel px-3 py-1.5 text-sm outline-none placeholder:text-ink-muted focus:border-accent"
          />
          <button
            class="rounded-lg border border-line px-3 py-1.5 text-sm font-medium text-ink-muted transition-colors hover:bg-panel-muted hover:text-ink"
            :disabled="!selectable.length"
            @click="selectAllVisible"
          >
            {{ t('apps.selectAll') }}
          </button>
          <button
            class="rounded-lg border border-line px-3 py-1.5 text-sm font-medium text-ink-muted transition-colors hover:bg-panel-muted hover:text-ink"
            :disabled="!selection.appCount"
            @click="selection.clear()"
          >
            {{ t('apps.clear') }}
          </button>
        </div>

        <div class="mb-4 flex flex-wrap gap-1.5">
          <button
            class="rounded-full px-3 py-1 text-xs font-medium transition-colors"
            :class="
              category === null
                ? 'bg-accent text-accent-ink'
                : 'bg-panel-muted text-ink-muted hover:text-ink'
            "
            @click="category = null"
          >
            {{ t('apps.allCategories') }}
          </button>
          <button
            v-for="name in categories"
            :key="name"
            class="rounded-full px-3 py-1 text-xs font-medium transition-colors"
            :class="
              category === name
                ? 'bg-accent text-accent-ink'
                : 'bg-panel-muted text-ink-muted hover:text-ink'
            "
            @click="category = name"
          >
            {{ t(`category.${name}`) }}
          </button>
        </div>
      </template>
    </div>

    <!-- List -->
    <div v-if="data" class="min-h-0 flex-1 overflow-y-auto px-6 lg:px-8">
      <p v-if="!visible.length" class="py-8 text-center text-sm text-ink-muted">
        {{ t('apps.noResults') }}
      </p>

      <ul class="space-y-1.5 pb-4">
        <li v-for="app in visible" :key="app.id">
          <label
            class="flex cursor-pointer items-start gap-3 rounded-panel border border-line bg-panel p-3 transition-colors"
            :class="
              app.installable && !app.installed
                ? 'hover:border-accent/50'
                : 'cursor-not-allowed opacity-60'
            "
          >
            <input
              type="checkbox"
              class="mt-0.5 size-4 shrink-0 accent-[var(--accent)]"
              :checked="selection.isAppSelected(app.id)"
              :disabled="!app.installable || app.installed"
              @change="selection.toggleApp(app.id)"
            />
            <span class="min-w-0 flex-1">
              <span class="flex flex-wrap items-center gap-2">
                <span class="font-medium">{{ app.name }}</span>
                <span
                  v-if="app.installed"
                  class="rounded-full bg-success/15 px-2 py-0.5 text-xs font-medium text-success"
                >
                  {{ t('apps.installed') }}
                </span>
                <span
                  v-else-if="!app.installable"
                  class="rounded-full bg-panel-muted px-2 py-0.5 text-xs text-ink-muted"
                >
                  {{ t('apps.unavailable') }}
                </span>
                <span v-else-if="app.provider" class="text-xs text-ink-muted">
                  {{ app.provider }}
                </span>
              </span>
              <span v-if="app.description" class="mt-0.5 block text-sm text-pretty text-ink-muted">
                {{ app.description }}
              </span>
            </span>
          </label>
        </li>
      </ul>
    </div>

    <!-- Action bar -->
    <div
      v-if="data && (selection.appCount || installing || report || installError)"
      class="shrink-0 border-t border-line bg-panel px-6 py-3 lg:px-8"
    >
      <div v-if="installing" class="space-y-2">
        <p class="flex items-center gap-2 text-sm font-medium">
          <span
            class="size-3 shrink-0 animate-spin rounded-full border-2 border-accent border-t-transparent"
            aria-hidden="true"
          />
          {{ currentItem || t('apps.installing') }}
        </p>
        <pre
          v-if="logLines.length"
          class="selectable max-h-32 overflow-y-auto rounded-lg bg-canvas p-2 font-mono text-xs text-ink-muted"
        >{{ logLines.join('\n') }}</pre>
      </div>

      <div v-else-if="installError" class="text-sm" role="alert">
        <p class="font-medium text-danger">{{ installError }}</p>
        <button
          class="mt-2 rounded-lg border border-line px-3 py-1.5 text-sm font-medium"
          @click="installError = null"
        >
          {{ t('apps.close') }}
        </button>
      </div>

      <div v-else-if="report" class="space-y-2">
        <p class="text-sm font-medium">
          {{
            t('apps.summary', {
              ok: report.items.filter((i) => i.status !== 'failed').length,
              total: report.items.length,
            })
          }}
        </p>
        <p
          v-if="report.items.some((i) => i.rebootRequired)"
          class="flex items-center gap-1.5 text-sm text-warning"
        >
          <AppIcon name="warning" :size="14" />
          {{ t('apps.rebootRequired') }}
        </p>
        <ul class="max-h-32 space-y-1 overflow-y-auto text-sm">
          <li v-for="item in report.items" :key="item.appId" class="flex flex-wrap gap-2">
            <span
              :class="{
                'text-success': item.status === 'success',
                'text-ink-muted': item.status === 'skipped',
                'text-danger': item.status === 'failed',
              }"
            >
              {{ statusOf(item) }}
            </span>
            <span>{{ item.displayName }}</span>
            <span v-if="detailOf(item)" class="selectable text-xs text-ink-muted">
              {{ detailOf(item) }}
            </span>
          </li>
        </ul>
        <button
          class="rounded-lg border border-line px-3 py-1.5 text-sm font-medium"
          @click="report = null"
        >
          {{ t('apps.close') }}
        </button>
      </div>

      <div v-else class="flex flex-wrap items-center justify-between gap-3">
        <span class="text-sm text-ink-muted">
          {{ t('apps.selected', { count: selection.appCount }) }}
        </span>
        <button
          class="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-ink transition-opacity hover:opacity-90 disabled:opacity-50"
          :disabled="!selection.appCount"
          @click="runInstall"
        >
          {{ t('apps.install', { count: selection.appCount }) }}
        </button>
      </div>
    </div>
  </div>
</template>
