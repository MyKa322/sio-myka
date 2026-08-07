<script setup lang="ts">
defineProps<{
  title: string
  /** Optional short value shown at top-right, e.g. a percentage. */
  badge?: string
}>()
</script>

<template>
  <section
    class="rounded-panel border border-line bg-panel p-4 transition-colors"
    :aria-label="title"
  >
    <header class="mb-3 flex items-baseline justify-between gap-3">
      <h2 class="text-sm font-semibold tracking-wide text-ink-muted uppercase">
        {{ title }}
      </h2>
      <span v-if="badge" class="text-sm font-medium text-ink-muted tabular-nums">
        {{ badge }}
      </span>
    </header>
    <slot />
  </section>
</template>
