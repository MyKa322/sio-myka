import { beforeEach, describe, expect, it } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useSelectionStore } from './selection'

describe('selection store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('toggles an app on and off', () => {
    const store = useSelectionStore()

    store.toggleApp('firefox')
    expect(store.isAppSelected('firefox')).toBe(true)
    expect(store.appCount).toBe(1)

    store.toggleApp('firefox')
    expect(store.isAppSelected('firefox')).toBe(false)
    expect(store.appCount).toBe(0)
  })

  it('does not double-count the same app', () => {
    const store = useSelectionStore()
    store.setApps(['firefox', 'firefox', 'vlc'])
    expect(store.appCount).toBe(2)
  })

  it('exposes selection as a stable array for the install command', () => {
    const store = useSelectionStore()
    store.setApps(['firefox', 'vlc'])
    expect(store.appIds).toEqual(['firefox', 'vlc'])
  })

  it('adds without discarding an existing selection', () => {
    // Applying a profile must not wipe what the user already ticked.
    const store = useSelectionStore()
    store.setApps(['firefox'])
    store.addApps(['vlc', '7zip'])
    expect([...store.appIds].sort()).toEqual(['7zip', 'firefox', 'vlc'])
  })

  it('setApps replaces rather than merges', () => {
    const store = useSelectionStore()
    store.setApps(['firefox'])
    store.setApps(['vlc'])
    expect(store.appIds).toEqual(['vlc'])
  })

  it('reports emptiness across both apps and tweaks', () => {
    const store = useSelectionStore()
    expect(store.isEmpty).toBe(true)

    store.setTweaks(['privacy.telemetry.disable'])
    expect(store.isEmpty).toBe(false)
    expect(store.tweakCount).toBe(1)

    store.clear()
    expect(store.isEmpty).toBe(true)
    expect(store.appCount).toBe(0)
    expect(store.tweakCount).toBe(0)
  })

  it('reacts to toggling so computed values update', () => {
    const store = useSelectionStore()
    expect(store.appCount).toBe(0)
    store.toggleApp('a')
    expect(store.appCount).toBe(1)
    store.toggleApp('b')
    expect(store.appIds.sort()).toEqual(['a', 'b'])
  })
})
