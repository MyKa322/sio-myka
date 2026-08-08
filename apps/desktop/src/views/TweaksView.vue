<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery } from '@tanstack/vue-query'
import type { UnlistenFn } from '@tauri-apps/api/event'

import AppIcon from '@/components/AppIcon.vue'
import PageHeader from '@/components/PageHeader.vue'
import {
  applyTweaks,
  listJournal,
  listTweaks,
  onTweakProgress,
  revertTweak,
  type IpcError,
} from '@/lib/ipc'
import type { ApplyReport, TweakView } from '@/lib/types'

const { t, locale } = useI18n()

const { data, isPending, error, refetch } = useQuery({
  queryKey: computed(() => ['tweaks', locale.value]),
  queryFn: () => listTweaks(locale.value),
})

/** Tweak ids with an un-reverted journal entry, i.e. things we can undo. */
const undoable = ref<Set<string>>(new Set())
async function refreshJournal() {
  const entries = await listJournal().catch(() => [])
  undoable.value = new Set(
    entries.filter((e) => e.reverted_at == null).map((e) => e.tweak_id),
  )
}
refreshJournal()

const selected = ref<Set<string>>(new Set())
function toggle(id: string) {
  const next = new Set(selected.value)
  if (!next.delete(id)) next.add(id)
  selected.value = next
}

const applying = ref(false)
const reverting = ref<string | null>(null)
const needsConfirm = ref(false)
const report = ref<ApplyReport | null>(null)
const failure = ref<string | null>(null)
const revertNotice = ref<string | null>(null)
let unlisten: UnlistenFn | null = null

const grouped = computed(() => {
  const groups = new Map<string, TweakView[]>()
  for (const tweak of data.value ?? []) {
    const list = groups.get(tweak.category) ?? []
    list.push(tweak)
    groups.set(tweak.category, list)
  }
  return [...groups.entries()].sort(([a], [b]) => a.localeCompare(b))
})

/** Selected tweaks that need an explicit yes before anything happens. */
const risky = computed(() =>
  (data.value ?? []).filter(
    (tweak) => selected.value.has(tweak.id) && (tweak.risk !== 'low' || tweak.irreversible),
  ),
)

function statusClass(status: TweakView['status']): string {
  return {
    applied: 'bg-success/15 text-success',
    partial: 'bg-warning/15 text-warning',
    not_applied: 'bg-panel-muted text-ink-muted',
    unknown: 'bg-panel-muted text-ink-muted',
  }[status]
}

function requestApply() {
  failure.value = null
  report.value = null
  if (risky.value.length) {
    needsConfirm.value = true
    return
  }
  void run()
}

async function run() {
  needsConfirm.value = false
  applying.value = true

  unlisten = await onTweakProgress(() => {
    // Tweaks are quick; the per-item log is not useful here, but subscribing keeps
    // the backend's forwarder alive and the channel drained.
  })

  try {
    report.value = await applyTweaks([...selected.value])
    selected.value = new Set()
    await Promise.all([refetch(), refreshJournal()])
  } catch (thrown) {
    failure.value = t((thrown as IpcError).translationKey ?? 'errors.unknown')
  } finally {
    unlisten?.()
    unlisten = null
    applying.value = false
  }
}

async function undo(id: string) {
  reverting.value = id
  failure.value = null
  revertNotice.value = null
  try {
    const result = await revertTweak(id)
    if (result.failures.length) revertNotice.value = t('tweaks.revertFailed')
    else if (result.irreversible.length) {
      revertNotice.value = t('tweaks.revertIrreversible', {
        items: result.irreversible.join(', '),
      })
    }
    await Promise.all([refetch(), refreshJournal()])
  } catch (thrown) {
    failure.value = t((thrown as IpcError).translationKey ?? 'errors.unknown')
  } finally {
    reverting.value = null
  }
}

const restorePointMessage = computed(() => {
  const point = report.value?.restorePoint
  if (!point) return null
  return {
    created: t('tweaks.restorePointCreated'),
    skipped_throttled: t('tweaks.restorePointThrottled'),
    skipped_disabled: t('tweaks.restorePointDisabled'),
  }[point.outcome]
})

onUnmounted(() => unlisten?.())
</script>

<template>
  <div class="flex h-full flex-col">
    <div class="shrink-0 p-6 pb-0 lg:p-8 lg:pb-0">
      <PageHeader :title="t('tweaks.title')" :subtitle="t('tweaks.subtitle')" />

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

      <p v-else-if="!data?.length" class="text-sm text-ink-muted">{{ t('tweaks.empty') }}</p>

      <div
        v-if="failure"
        class="mb-4 rounded-panel border border-danger/40 bg-danger/10 p-3 text-sm"
        role="alert"
      >
        {{ failure }}
      </div>
      <div
        v-else-if="revertNotice"
        class="mb-4 rounded-panel border border-warning/40 bg-warning/10 p-3 text-sm"
        role="status"
      >
        {{ revertNotice }}
      </div>
    </div>

    <div v-if="data?.length" class="min-h-0 flex-1 overflow-y-auto px-6 lg:px-8">
      <section v-for="[category, items] in grouped" :key="category" class="mb-6">
        <h2 class="mb-2 text-sm font-semibold tracking-wide text-ink-muted uppercase">
          {{ t(`category.${category}`, category) }}
        </h2>

        <ul class="space-y-1.5">
          <li v-for="tweak in items" :key="tweak.id">
            <div class="rounded-panel border border-line bg-panel p-3">
              <div class="flex items-start gap-3">
                <input
                  :id="`tweak-${tweak.id}`"
                  type="checkbox"
                  class="mt-1 size-4 shrink-0 accent-[var(--accent)]"
                  :checked="selected.has(tweak.id)"
                  :disabled="tweak.status === 'applied'"
                  @change="toggle(tweak.id)"
                />
                <div class="min-w-0 flex-1">
                  <label
                    :for="`tweak-${tweak.id}`"
                    class="flex cursor-pointer flex-wrap items-center gap-2"
                  >
                    <span class="font-medium">{{ tweak.name }}</span>
                    <span
                      class="rounded-full px-2 py-0.5 text-xs font-medium"
                      :class="statusClass(tweak.status)"
                    >
                      {{ t(`tweakStatus.${tweak.status}`) }}
                    </span>
                    <span
                      v-if="tweak.risk !== 'low'"
                      class="rounded-full bg-warning/15 px-2 py-0.5 text-xs font-medium text-warning"
                    >
                      {{ t(`risk.${tweak.risk}`) }}
                    </span>
                    <span
                      v-if="tweak.irreversible"
                      class="flex items-center gap-1 text-xs text-ink-muted"
                    >
                      <AppIcon name="warning" :size="12" />
                    </span>
                  </label>
                  <p v-if="tweak.description" class="mt-0.5 text-sm text-pretty text-ink-muted">
                    {{ tweak.description }}
                  </p>
                </div>

                <button
                  v-if="undoable.has(tweak.id)"
                  class="shrink-0 rounded-lg border border-line px-2.5 py-1 text-xs font-medium text-ink-muted transition-colors hover:border-accent hover:text-ink disabled:opacity-50"
                  :disabled="reverting === tweak.id"
                  @click="undo(tweak.id)"
                >
                  {{ reverting === tweak.id ? t('tweaks.reverting') : t('tweaks.revert') }}
                </button>
              </div>
            </div>
          </li>
        </ul>
      </section>
    </div>

    <!-- Action bar -->
    <div
      v-if="data?.length && (selected.size || applying || report || needsConfirm)"
      class="shrink-0 border-t border-line bg-panel px-6 py-3 lg:px-8"
    >
      <div v-if="applying" class="flex items-center gap-2 text-sm font-medium">
        <span
          class="size-3 animate-spin rounded-full border-2 border-accent border-t-transparent"
          aria-hidden="true"
        />
        {{ t('tweaks.applying') }}
      </div>

      <!-- Anything above "safe", or anything that cannot be undone, gets an explicit
           confirmation naming exactly what it is. -->
      <div v-else-if="needsConfirm" class="space-y-2">
        <p class="text-sm font-medium">{{ t('tweaks.confirmTitle') }}</p>
        <p v-if="risky.some((r) => r.risk !== 'low')" class="text-sm text-warning">
          {{ t('tweaks.confirmRisk') }}
        </p>
        <p v-if="risky.some((r) => r.irreversible)" class="text-sm text-warning">
          {{ t('tweaks.confirmIrreversible') }}
        </p>
        <ul class="text-sm text-ink-muted">
          <li v-for="item in risky" :key="item.id">· {{ item.name }}</li>
        </ul>
        <div class="flex gap-2">
          <button
            class="rounded-lg bg-warning px-3 py-1.5 text-sm font-medium text-canvas transition-opacity hover:opacity-90"
            @click="run"
          >
            {{ t('tweaks.confirmYes') }}
          </button>
          <button
            class="rounded-lg border border-line px-3 py-1.5 text-sm font-medium"
            @click="needsConfirm = false"
          >
            {{ t('common.cancel') }}
          </button>
        </div>
      </div>

      <div v-else-if="report" class="space-y-2">
        <p class="text-sm font-medium">
          {{
            t('tweaks.summary', {
              ok: report.items.filter((i) => i.status !== 'failed').length,
              total: report.items.length,
            })
          }}
        </p>
        <p v-if="restorePointMessage" class="text-sm text-pretty text-ink-muted">
          {{ restorePointMessage }}
        </p>
        <p v-if="report.restartRequired" class="flex items-center gap-1.5 text-sm text-warning">
          <AppIcon name="warning" :size="14" />
          {{ t('tweaks.restartRequired') }}
        </p>
        <ul
          v-if="report.items.some((i) => i.status === 'failed')"
          class="max-h-24 space-y-1 overflow-y-auto text-sm text-danger"
        >
          <li v-for="item in report.items.filter((i) => i.status === 'failed')" :key="item.tweakId">
            {{ item.tweakId }}
          </li>
        </ul>
        <button
          class="rounded-lg border border-line px-3 py-1.5 text-sm font-medium"
          @click="report = null"
        >
          {{ t('tweaks.close') }}
        </button>
      </div>

      <div v-else class="flex flex-wrap items-center justify-between gap-3">
        <span class="text-sm text-ink-muted">
          {{ t('tweaks.selected', { count: selected.size }) }}
        </span>
        <span class="flex gap-2">
          <button
            class="rounded-lg border border-line px-3 py-1.5 text-sm font-medium text-ink-muted transition-colors hover:bg-panel-muted hover:text-ink"
            @click="selected = new Set()"
          >
            {{ t('tweaks.clear') }}
          </button>
          <button
            class="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-ink transition-opacity hover:opacity-90 disabled:opacity-50"
            :disabled="!selected.size"
            @click="requestApply"
          >
            {{ t('tweaks.apply', { count: selected.size }) }}
          </button>
        </span>
      </div>
    </div>
  </div>
</template>
