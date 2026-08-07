<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import AppIcon from '@/components/AppIcon.vue'

defineProps<{ icon: string; description: string }>()

const { t } = useI18n()
</script>

<template>
  <div
    class="rounded-panel border border-dashed border-line bg-panel/50 px-6 py-14 text-center"
  >
    <span
      class="mx-auto mb-4 grid size-12 place-items-center rounded-full bg-panel-muted text-ink-muted"
    >
      <AppIcon :name="icon" :size="24" />
    </span>
    <h2 class="text-base font-medium">{{ t('placeholder.title') }}</h2>
    <p class="mx-auto mt-2 max-w-md text-sm text-balance text-ink-muted">
      {{ description }}
    </p>
  </div>
</template>
