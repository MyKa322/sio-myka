<script setup lang="ts">
import { computed } from 'vue'
import { usageLevel } from '@/lib/format'

const props = defineProps<{
  /** 0–100. */
  percent: number
  label?: string
}>()

const clamped = computed(() => Math.max(0, Math.min(100, props.percent)))

const barColor = computed(
  () =>
    ({
      ok: 'bg-accent',
      warn: 'bg-warning',
      critical: 'bg-danger',
    })[usageLevel(clamped.value)],
)
</script>

<template>
  <div
    class="h-2 w-full overflow-hidden rounded-full bg-panel-muted"
    role="progressbar"
    :aria-valuenow="Math.round(clamped)"
    aria-valuemin="0"
    aria-valuemax="100"
    :aria-label="label"
  >
    <div
      class="h-full rounded-full transition-[width] duration-500 ease-out"
      :class="barColor"
      :style="{ width: `${clamped}%` }"
    />
  </div>
</template>
