<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery } from '@tanstack/vue-query'

import AppIcon from '@/components/AppIcon.vue'
import PageHeader from '@/components/PageHeader.vue'
import StatCard from '@/components/StatCard.vue'
import UsageBar from '@/components/UsageBar.vue'
import { formatBytes, formatPercent } from '@/lib/format'
import { systemSnapshot, type IpcError } from '@/lib/ipc'

const { t, locale } = useI18n()

const { data, isPending, error, refetch, isFetching } = useQuery({
  queryKey: ['system-snapshot'],
  queryFn: systemSnapshot,
})

const ipcError = computed(() => error.value as IpcError | null)

const memoryPercent = computed(() => {
  const memory = data.value?.memory
  if (!memory || memory.total_bytes === 0) return 0
  return (memory.used_bytes / memory.total_bytes) * 100
})

const coreLabel = computed(() => {
  const cpu = data.value?.cpu
  if (!cpu) return ''
  return cpu.physical_cores
    ? t('dashboard.cores', { physical: cpu.physical_cores, logical: cpu.logical_cores })
    : t('dashboard.coresLogicalOnly', { logical: cpu.logical_cores })
})

/** Fixed drives first, then removable — the system drive is what people look for. */
const disks = computed(() =>
  [...(data.value?.disks ?? [])].sort(
    (a, b) => Number(a.is_removable) - Number(b.is_removable) ||
      a.mount_point.localeCompare(b.mount_point),
  ),
)

function diskPercent(total: number, available: number): number {
  if (total === 0) return 0
  return ((total - available) / total) * 100
}
</script>

<template>
  <div class="p-6 lg:p-8">
    <PageHeader :title="t('dashboard.title')" :subtitle="t('dashboard.subtitle')" />

    <p v-if="isPending" class="text-sm text-ink-muted">{{ t('common.loading') }}</p>

    <div
      v-else-if="ipcError"
      class="rounded-panel border border-danger/40 bg-danger/10 p-4"
      role="alert"
    >
      <p class="text-sm font-medium">{{ t(ipcError.translationKey) }}</p>
      <p class="selectable mt-1 font-mono text-xs break-all text-ink-muted">
        {{ ipcError.detail }}
      </p>
      <button
        class="mt-3 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink transition-opacity hover:opacity-90"
        :disabled="isFetching"
        @click="refetch()"
      >
        {{ t('common.retry') }}
      </button>
    </div>

    <div v-else-if="data" class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
      <StatCard :title="t('dashboard.operatingSystem')" class="md:col-span-2 xl:col-span-1">
        <div class="flex items-start gap-3">
          <AppIcon name="windows" :size="28" class="mt-0.5 shrink-0 text-accent" />
          <div class="min-w-0">
            <p class="truncate font-medium">{{ data.os.edition }}</p>
            <dl class="mt-2 space-y-1 text-sm text-ink-muted">
              <div class="flex justify-between gap-4">
                <dt>{{ t('dashboard.version') }}</dt>
                <dd class="text-ink tabular-nums">
                  {{ data.os.display_version }} ({{ data.os.build }})
                </dd>
              </div>
              <div class="flex justify-between gap-4">
                <dt>{{ t('dashboard.architecture') }}</dt>
                <dd class="text-ink">{{ data.os.arch }}</dd>
              </div>
              <div class="flex justify-between gap-4">
                <dt>{{ t('dashboard.machineName') }}</dt>
                <dd class="truncate text-ink">{{ data.os.machine_name }}</dd>
              </div>
              <div class="flex justify-between gap-4">
                <dt>{{ t('dashboard.activation') }}</dt>
                <dd
                  :class="
                    data.os.activation === 'licensed'
                      ? 'text-success'
                      : data.os.activation === 'unknown'
                        ? 'text-ink-muted'
                        : 'text-warning'
                  "
                >
                  {{ t(`activation.${data.os.activation}`) }}
                </dd>
              </div>
            </dl>
          </div>
        </div>
      </StatCard>

      <StatCard :title="t('dashboard.processor')">
        <div class="flex items-start gap-3">
          <AppIcon name="cpu" :size="28" class="mt-0.5 shrink-0 text-accent" />
          <div class="min-w-0">
            <p class="font-medium text-pretty">{{ data.cpu.brand }}</p>
            <p class="mt-1 text-sm text-ink-muted tabular-nums">{{ coreLabel }}</p>
          </div>
        </div>
      </StatCard>

      <StatCard
        :title="t('dashboard.memory')"
        :badge="formatPercent(memoryPercent, locale)"
      >
        <div class="flex items-start gap-3">
          <AppIcon name="memory" :size="28" class="mt-0.5 shrink-0 text-accent" />
          <div class="min-w-0 flex-1">
            <p class="font-medium tabular-nums">
              {{ formatBytes(data.memory.used_bytes, locale) }} /
              {{ formatBytes(data.memory.total_bytes, locale) }}
            </p>
            <UsageBar
              class="mt-3"
              :percent="memoryPercent"
              :label="t('dashboard.memory')"
            />
          </div>
        </div>
      </StatCard>

      <StatCard :title="t('dashboard.graphics')">
        <div class="flex items-start gap-3">
          <AppIcon name="gpu" :size="28" class="mt-0.5 shrink-0 text-accent" />
          <div class="min-w-0 flex-1">
            <p v-if="!data.gpus.length" class="text-sm text-ink-muted">
              {{ t('dashboard.noGpuDetected') }}
            </p>
            <ul v-else class="space-y-2">
              <li v-for="gpu in data.gpus" :key="gpu.name" class="min-w-0">
                <p class="truncate font-medium" :title="gpu.name">{{ gpu.name }}</p>
                <p v-if="gpu.vram_bytes" class="text-sm text-ink-muted tabular-nums">
                  {{ formatBytes(gpu.vram_bytes, locale) }}
                </p>
              </li>
            </ul>
          </div>
        </div>
      </StatCard>

      <StatCard
        :title="t('dashboard.storage')"
        class="md:col-span-2 xl:col-span-3"
      >
        <ul class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          <li v-for="disk in disks" :key="disk.mount_point" class="min-w-0">
            <div class="mb-1.5 flex items-baseline justify-between gap-3">
              <span class="flex items-center gap-2 font-medium">
                <AppIcon name="disk" :size="16" class="text-ink-muted" />
                {{ disk.mount_point }}
              </span>
              <span class="text-sm text-ink-muted tabular-nums">
                {{ formatBytes(disk.available_bytes, locale) }} {{ t('common.free') }}
              </span>
            </div>
            <UsageBar
              :percent="diskPercent(disk.total_bytes, disk.available_bytes)"
              :label="disk.mount_point"
            />
            <p class="mt-1.5 text-xs text-ink-muted tabular-nums">
              {{ formatBytes(disk.total_bytes - disk.available_bytes, locale) }} /
              {{ formatBytes(disk.total_bytes, locale) }}
            </p>
            <p
              v-if="!disk.is_removable && disk.available_bytes < 10 * 1024 ** 3"
              class="mt-1.5 flex items-start gap-1.5 text-xs text-warning"
            >
              <AppIcon name="warning" :size="14" class="mt-px shrink-0" />
              {{ t('dashboard.lowSpaceWarning') }}
            </p>
          </li>
        </ul>
      </StatCard>
    </div>
  </div>
</template>
