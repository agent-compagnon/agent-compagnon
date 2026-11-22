// The entries of the journal the window holds: those of `get_journal`, then each `journal-entry`. In pause, the list
// shown does not change and the new entries wait; resuming shows them. Shown and waiting entries together are
// bounded: beyond the limit, the oldest go, pause or not. After `journal-resync` (entries lost on the way), the
// whole journal is asked again and merged by `seq`.

import { shallowReactive, shallowReadonly } from 'vue'
import { type Backend, InvalidJournal, type JournalEvent, reason } from '@/api/backend'
import type { JournalEntry } from '@/api/types'
import { JOURNAL_LIMIT, merge } from '@/components/journal/model'

export type JournalPhase = 'loading' | 'ready' | 'unavailable' | 'invalid'

export type ExportState =
  | { phase: 'idle' }
  | { phase: 'running' }
  | { phase: 'saved'; path: string }
  | { phase: 'failed'; reason: string }

export interface JournalState {
  phase: JournalPhase
  /** What the list shows, oldest first. */
  shown: readonly JournalEntry[]
  /** Arrived during the pause, oldest first. */
  pending: readonly JournalEntry[]
  paused: boolean
  export: ExportState
}

export function createJournal(backend: Backend, limit = JOURNAL_LIMIT) {
  const state = shallowReactive<JournalState>({
    phase: 'loading',
    shown: [],
    pending: [],
    paused: false,
    export: { phase: 'idle' },
  })
  // Every `seq` at or below this one was dropped by the bound: a resynchronisation does not bring it back.
  let floor = 0
  // The `seq` of every entry shown or waiting.
  const held = new Set<number>()
  let stop: (() => void) | null = null
  let resyncing: Promise<void> | null = null
  let again = false

  /** The oldest go until shown and waiting entries fit; those shown first, being the oldest. */
  function bound() {
    let excess = state.shown.length + state.pending.length - limit
    if (excess <= 0) return
    const fromShown = Math.min(excess, state.shown.length)
    const dropped = state.shown.slice(0, fromShown)
    state.shown = state.shown.slice(fromShown)
    excess -= fromShown
    const droppedPending = state.pending.slice(0, excess)
    state.pending = state.pending.slice(excess)
    for (const entry of [...dropped, ...droppedPending]) {
      floor = Math.max(floor, entry.seq)
      held.delete(entry.seq)
    }
  }

  function add(entries: readonly JournalEntry[]) {
    const fresh = entries.filter((e) => e.seq > floor && !held.has(e.seq))
    if (fresh.length === 0) return
    for (const entry of fresh) held.add(entry.seq)
    if (state.paused) state.pending = merge(state.pending, fresh)
    else state.shown = merge(state.shown, fresh)
    bound()
  }

  async function load(): Promise<void> {
    try {
      add(await backend.getJournal())
      state.phase = 'ready'
    } catch (error) {
      state.phase = error instanceof InvalidJournal ? 'invalid' : 'unavailable'
    }
  }

  /** One resynchronisation at a time; one more if another was asked meanwhile. */
  function resync(): Promise<void> {
    if (resyncing) {
      again = true
      return resyncing
    }
    // Read through a function: asked meanwhile by another call, which the narrowing of `again` does not see.
    const askedAgain = () => again
    resyncing = (async () => {
      do {
        again = false
        await load()
      } while (askedAgain())
      resyncing = null
    })()
    return resyncing
  }

  function handle(event: JournalEvent) {
    if (event.type === 'entry') add([event.entry])
    else void resync()
  }

  async function start(): Promise<void> {
    stop = await backend.onJournal(handle)
    await resync()
  }

  function close() {
    stop?.()
    stop = null
  }

  function pause() {
    state.paused = true
  }

  function resume() {
    state.shown = merge(state.shown, state.pending)
    state.pending = []
    state.paused = false
  }

  async function exportDiagnostic(): Promise<void> {
    if (state.export.phase === 'running') return
    const before = state.export
    state.export = { phase: 'running' }
    try {
      const outcome = await backend.exportDiagnostic()
      // Closing the dialog is no news: whatever was said before stays.
      state.export = outcome.status === 'saved' ? { phase: 'saved', path: outcome.path } : before
    } catch (error) {
      state.export = { phase: 'failed', reason: reason(error) }
    }
  }

  return {
    state: shallowReadonly(state),
    start,
    close,
    pause,
    resume,
    resync,
    exportDiagnostic,
    dismissExport: () => {
      state.export = { phase: 'idle' }
    },
  }
}

export type JournalStore = ReturnType<typeof createJournal>
