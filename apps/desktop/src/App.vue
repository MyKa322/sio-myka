<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import AppIcon from '@/components/AppIcon.vue'

const { t } = useI18n()

const NAV = [
  { to: '/', key: 'dashboard', icon: 'dashboard' },
  { to: '/apps', key: 'apps', icon: 'apps' },
  { to: '/tweaks', key: 'tweaks', icon: 'tweaks' },
  { to: '/profiles', key: 'profiles', icon: 'profiles' },
  { to: '/settings', key: 'settings', icon: 'settings' },
] as const
</script>

<template>
  <div class="flex h-full">
    <!-- Sidebar. Collapses to icons on narrow windows; labels stay in the accessible
         name via `title`, so it degrades without becoming unusable. -->
    <nav
      class="flex w-16 shrink-0 flex-col gap-1 border-r border-line bg-panel p-2 lg:w-56 lg:p-3"
      :aria-label="t('app.name')"
    >
      <div class="mb-4 flex items-center gap-2.5 px-1.5 pt-1.5">
        <span
          class="grid size-8 shrink-0 place-items-center rounded-lg bg-accent text-accent-ink"
          aria-hidden="true"
        >
          <AppIcon name="apps" :size="18" />
        </span>
        <span class="hidden min-w-0 flex-col lg:flex">
          <span class="truncate text-sm leading-tight font-semibold">{{ t('app.name') }}</span>
          <span class="truncate text-xs leading-tight text-ink-muted">
            {{ t('app.tagline') }}
          </span>
        </span>
      </div>

      <!-- `exact-active-class` rather than `active-class`: "/" is a prefix of every
           other route and would otherwise stay highlighted everywhere. -->
      <RouterLink
        v-for="item in NAV"
        :key="item.to"
        :to="item.to"
        :title="t(`nav.${item.key}`)"
        class="flex items-center gap-3 rounded-lg px-2.5 py-2 text-sm font-medium text-ink-muted transition-colors hover:bg-panel-muted hover:text-ink lg:px-3"
        exact-active-class="bg-accent! text-accent-ink! hover:bg-accent!"
      >
        <AppIcon :name="item.icon" class="shrink-0" />
        <span class="hidden truncate lg:inline">{{ t(`nav.${item.key}`) }}</span>
      </RouterLink>
    </nav>

    <main class="min-w-0 flex-1 overflow-y-auto">
      <RouterView v-slot="{ Component }">
        <component :is="Component" />
      </RouterView>
    </main>
  </div>
</template>
