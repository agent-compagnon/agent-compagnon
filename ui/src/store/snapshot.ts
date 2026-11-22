// The last snapshot of the application, kept up to date by `snapshot-changed`, and the commands of the window.
// Nothing here is secret: the snapshot holds none, and the commands send identifiers only.

import { type InjectionKey, inject, reactive, shallowReadonly } from 'vue'
import { type Backend, reason } from '@/api/backend'
import type { AppSnapshot } from '@/api/types'

export type Phase = 'loading' | 'ready' | 'unavailable' | 'invalid'

export interface StoreState {
  phase: Phase
  snapshot: AppSnapshot | null
  /** Why the last command was refused, in French; `null` when it passed. */
  commandError: string | null
}

export function createStore(backend: Backend) {
  const state = reactive<StoreState>({ phase: 'loading', snapshot: null, commandError: null })
  let stop: (() => void) | null = null
  // A snapshot from the event is never replaced by an older answer of `get_snapshot`.
  let received = 0

  function accept(snapshot: AppSnapshot | null) {
    received += 1
    if (snapshot === null) {
      state.phase = 'invalid'
      state.snapshot = null
      return
    }
    state.snapshot = snapshot
    state.phase = 'ready'
  }

  async function start(): Promise<void> {
    stop = await backend.onSnapshot(accept)
    const before = received
    try {
      const snapshot = await backend.getSnapshot()
      if (received === before) accept(snapshot)
    } catch {
      if (received === before) state.phase = 'unavailable'
    }
  }

  function close() {
    stop?.()
    stop = null
  }

  async function run(command: () => Promise<void>): Promise<boolean> {
    state.commandError = null
    try {
      await command()
      return true
    } catch (error) {
      state.commandError = reason(error)
      return false
    }
  }

  return {
    state: shallowReadonly(state),
    start,
    close,
    connect: (session: string) => run(() => backend.connectSession(session)),
    disconnect: (session: string) => run(() => backend.disconnectSession(session)),
    setProfile: (profile: string) => run(() => backend.setProfile(profile)),
    resume: () => run(() => backend.resumeGateway()),
    showConfirmations: () => run(() => backend.showConfirmations()),
    dismissError: () => {
      state.commandError = null
    },
  }
}

export type Store = ReturnType<typeof createStore>

export const STORE: InjectionKey<Store> = Symbol('store')

export function useStore(): Store {
  const store = inject(STORE)
  if (store === undefined) throw new Error('store not provided')
  return store
}
