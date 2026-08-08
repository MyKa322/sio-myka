<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

import AppIcon from '@/components/AppIcon.vue'
import PageHeader from '@/components/PageHeader.vue'
import StatCard from '@/components/StatCard.vue'
import {
  deleteProfile,
  listProfiles,
  revealProfilesFolder,
  saveProfile,
  type IpcError,
} from '@/lib/ipc'
import type { Profile } from '@/lib/types'
import { useSelectionStore } from '@/stores/selection'

const { t, locale } = useI18n()
const router = useRouter()
const selection = useSelectionStore()

const profiles = ref<Profile[]>([])
const newName = ref('')
const notice = ref<string | null>(null)
const failure = ref<string | null>(null)

const canSave = computed(() => newName.value.trim().length > 0 && !selection.isEmpty)

async function refresh() {
  try {
    profiles.value = await listProfiles()
  } catch (thrown) {
    failure.value = t((thrown as IpcError).translationKey ?? 'errors.unknown')
  }
}

onMounted(refresh)

function formatDate(unixMs: number): string {
  return new Intl.DateTimeFormat(locale.value, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(unixMs))
}

async function save() {
  failure.value = null
  try {
    await saveProfile(newName.value.trim(), selection.appIds, selection.tweakIds)
    newName.value = ''
    await refresh()
  } catch (thrown) {
    failure.value = t((thrown as IpcError).translationKey ?? 'errors.unknown')
  }
}

async function apply(profile: Profile) {
  selection.setApps(profile.apps)
  selection.setTweaks(profile.tweaks)
  notice.value = t('profiles.applied')
  // Send them where the selection is actually visible and actionable.
  await router.push('/apps')
}

async function remove(profile: Profile) {
  failure.value = null
  try {
    await deleteProfile(profile.name)
    await refresh()
  } catch (thrown) {
    failure.value = t((thrown as IpcError).translationKey ?? 'errors.unknown')
  }
}

async function openFolder() {
  failure.value = null
  try {
    await revealProfilesFolder()
  } catch (thrown) {
    failure.value = t((thrown as IpcError).translationKey ?? 'errors.unknown')
  }
}
</script>

<template>
  <div class="p-6 lg:p-8">
    <PageHeader :title="t('profiles.title')" :subtitle="t('profiles.subtitle')" />

    <div class="grid max-w-3xl gap-4">
      <StatCard :title="t('profiles.save')">
        <p v-if="selection.isEmpty" class="mb-3 text-sm text-ink-muted">
          {{ t('profiles.nothingSelected') }}
        </p>
        <p v-else class="mb-3 text-sm text-ink-muted">
          {{ t('profiles.appsCount', { count: selection.appCount }) }} ·
          {{ t('profiles.tweaksCount', { count: selection.tweakCount }) }}
        </p>

        <div class="flex flex-wrap gap-2">
          <input
            v-model="newName"
            type="text"
            :placeholder="t('profiles.namePlaceholder')"
            class="selectable min-w-48 flex-1 rounded-lg border border-line bg-panel px-3 py-1.5 text-sm outline-none placeholder:text-ink-muted focus:border-accent"
            @keyup.enter="canSave && save()"
          />
          <button
            class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink transition-opacity hover:opacity-90 disabled:opacity-50"
            :disabled="!canSave"
            @click="save"
          >
            {{ t('profiles.save') }}
          </button>
        </div>
      </StatCard>

      <div
        v-if="failure"
        class="rounded-panel border border-danger/40 bg-danger/10 p-3 text-sm"
        role="alert"
      >
        {{ failure }}
      </div>
      <div
        v-else-if="notice"
        class="rounded-panel border border-success/40 bg-success/10 p-3 text-sm"
        role="status"
      >
        {{ notice }}
      </div>

      <StatCard :title="t('profiles.title')">
        <p v-if="!profiles.length" class="text-sm text-pretty text-ink-muted">
          {{ t('profiles.empty') }}
        </p>

        <ul v-else class="space-y-2">
          <li
            v-for="profile in profiles"
            :key="profile.name"
            class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-line p-3"
          >
            <span class="min-w-0">
              <span class="block truncate font-medium">{{ profile.name }}</span>
              <span class="block text-xs text-ink-muted">
                {{ t('profiles.appsCount', { count: profile.apps.length }) }} ·
                {{ t('profiles.tweaksCount', { count: profile.tweaks.length }) }} ·
                {{ formatDate(profile.created_at) }}
              </span>
            </span>
            <span class="flex shrink-0 gap-2">
              <button
                class="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink transition-opacity hover:opacity-90"
                @click="apply(profile)"
              >
                {{ t('profiles.apply') }}
              </button>
              <button
                class="rounded-lg border border-line px-3 py-1.5 text-sm font-medium text-ink-muted transition-colors hover:border-danger hover:text-danger"
                @click="remove(profile)"
              >
                {{ t('profiles.delete') }}
              </button>
            </span>
          </li>
        </ul>
      </StatCard>

      <StatCard :title="t('profiles.openFolder')">
        <p class="mb-3 text-sm text-pretty text-ink-muted">
          {{ t('profiles.openFolderHint') }}
        </p>
        <button
          class="flex items-center gap-2 rounded-lg border border-line px-3 py-1.5 text-sm font-medium transition-colors hover:bg-panel-muted"
          @click="openFolder"
        >
          <AppIcon name="profiles" :size="16" />
          {{ t('profiles.openFolder') }}
        </button>
      </StatCard>
    </div>
  </div>
</template>
