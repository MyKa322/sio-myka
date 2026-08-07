<script setup lang="ts">
import { computed } from 'vue'

/**
 * Inline SVG icons.
 *
 * Kept as raw path data rather than pulling in an icon package: the app needs about a
 * dozen glyphs, and a dependency plus its tree-shaking configuration is more moving
 * parts than the problem deserves. Paths are 24×24 stroke outlines.
 */
const PATHS: Record<string, string[]> = {
  dashboard: ['M3 3h7v9H3z', 'M14 3h7v5h-7z', 'M14 12h7v9h-7z', 'M3 16h7v5H3z'],
  apps: [
    'M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z',
    'm3.3 7 8.7 5 8.7-5',
    'M12 22V12',
  ],
  tweaks: [
    'M21 4h-7M10 4H3M21 12h-9M8 12H3M21 20h-5M12 20H3',
    'M14 2v4M8 10v4M16 18v4',
  ],
  profiles: [
    'M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z',
    'm2 12 9.17 4.16a2 2 0 0 0 1.66 0L22 12',
    'm2 17 9.17 4.16a2 2 0 0 0 1.66 0L22 17',
  ],
  settings: [
    'M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2Z',
    'M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z',
  ],
  cpu: [
    'M12 20v2M12 2v2M17 20v2M17 2v2M2 12h2M2 17h2M2 7h2M20 12h2M20 17h2M20 7h2M7 20v2M7 2v2',
    'M4 4h16v16H4z',
    'M8 8h8v8H8z',
  ],
  memory: ['M3 7h18v10H3z', 'M7 17v3M12 17v3M17 17v3', 'M7 11h2M11 11h2M15 11h2'],
  gpu: ['M2 6h20v11H2z', 'M6 17v3M18 17v3', 'M9.5 11.5a2.5 2.5 0 1 0 5 0 2.5 2.5 0 0 0-5 0Z'],
  disk: [
    'M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0Z',
    'M14 12a2 2 0 1 1-4 0 2 2 0 0 1 4 0Z',
    'm15.5 15.5 4 4',
  ],
  windows: ['M3 5.5 10 4.5v7H3zM11.5 4.3 21 3v8.5h-9.5zM3 12.5h7v7l-7-1zM11.5 12.5H21V21l-9.5-1.3z'],
  warning: ['M12 9v4M12 17h.01', 'm10.3 3.9-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.7-3l-8-14a2 2 0 0 0-3.4 0Z'],
  construction: ['M6 8h12M4 12h16M6 16h12', 'M3 4h18v16H3z'],
}

const props = defineProps<{ name: keyof typeof PATHS | string; size?: number }>()

const paths = computed(() => PATHS[props.name] ?? [])
</script>

<template>
  <svg
    :width="size ?? 20"
    :height="size ?? 20"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.75"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
  >
    <path v-for="(d, i) in paths" :key="i" :d="d" />
  </svg>
</template>
