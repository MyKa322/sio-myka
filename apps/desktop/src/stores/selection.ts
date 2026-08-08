import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

/**
 * What the user has ticked, shared between the Apps, Tuning and Profiles screens.
 *
 * Lives outside the views so a selection survives navigation — picking twenty apps and
 * then losing them by clicking "Profiles" to save them would be absurd.
 */
export const useSelectionStore = defineStore('selection', () => {
  const apps = ref(new Set<string>())
  const tweaks = ref(new Set<string>())

  const appCount = computed(() => apps.value.size)
  const tweakCount = computed(() => tweaks.value.size)
  const isEmpty = computed(() => apps.value.size === 0 && tweaks.value.size === 0)

  const appIds = computed(() => [...apps.value])
  const tweakIds = computed(() => [...tweaks.value])

  function toggleApp(id: string) {
    // Reassign rather than mutate: Vue does track Set mutations via reactive(), but a
    // fresh Set keeps computed invalidation obvious and cheap to reason about.
    const next = new Set(apps.value)
    if (!next.delete(id)) next.add(id)
    apps.value = next
  }

  function isAppSelected(id: string) {
    return apps.value.has(id)
  }

  function setApps(ids: Iterable<string>) {
    apps.value = new Set(ids)
  }

  function setTweaks(ids: Iterable<string>) {
    tweaks.value = new Set(ids)
  }

  /** Add without removing what is already ticked — used when applying a profile. */
  function addApps(ids: Iterable<string>) {
    const next = new Set(apps.value)
    for (const id of ids) next.add(id)
    apps.value = next
  }

  function clear() {
    apps.value = new Set()
    tweaks.value = new Set()
  }

  return {
    apps,
    tweaks,
    appCount,
    tweakCount,
    isEmpty,
    appIds,
    tweakIds,
    toggleApp,
    isAppSelected,
    setApps,
    setTweaks,
    addApps,
    clear,
  }
})
