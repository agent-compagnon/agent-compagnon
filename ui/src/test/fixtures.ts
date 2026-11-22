// Snapshots for the tests of the window: the scenarios of the simulated backend, at a fixed time.
import { type Scenario, createMockBackend, initialSnapshot, journalOf } from '@/api/mock'
import type { Backend, JournalEvent } from '@/api/backend'
import type { AppSnapshot, ExportOutcome, JournalEntry } from '@/api/types'
import { vi } from 'vitest'
import { type Component, defineComponent, h } from 'vue'
import { TooltipProvider } from '@/components/ui/tooltip'

/** Sun, 22 Nov 2026 14:00 UTC. */
export const NOW = Date.UTC(2026, 10, 22, 14, 0)

export function snapshotOf(name: Scenario): AppSnapshot {
  return initialSnapshot(name, NOW)
}

export function journalFixture(name: Scenario = 'sessions'): JournalEntry[] {
  return journalOf(name, NOW)
}

/** A backend whose commands are spies, answering with `snapshot` and `journal`. */
export function spyBackend(snapshot: AppSnapshot, journal: JournalEntry[] = []) {
  let listener: ((s: AppSnapshot | null) => void) | null = null
  let journalListener: ((e: JournalEvent) => void) | null = null
  const backend = {
    getSnapshot: vi.fn(() => Promise.resolve(structuredClone(snapshot))),
    onSnapshot: vi.fn((l: (s: AppSnapshot | null) => void) => {
      listener = l
      return Promise.resolve(() => {
        listener = null
      })
    }),
    connectSession: vi.fn(() => Promise.resolve()),
    disconnectSession: vi.fn(() => Promise.resolve()),
    setProfile: vi.fn(() => Promise.resolve()),
    resumeGateway: vi.fn(() => Promise.resolve()),
    showConfirmations: vi.fn(() => Promise.resolve()),
    setEnvironmentExposed: vi.fn<Backend['setEnvironmentExposed']>(() => Promise.resolve()),
    setMcpExposed: vi.fn<Backend['setMcpExposed']>(() => Promise.resolve()),
    acknowledgeMcpUpdate: vi.fn<Backend['acknowledgeMcpUpdate']>(() => Promise.resolve()),
    reloadMcp: vi.fn<Backend['reloadMcp']>(() => Promise.resolve()),
    getJournal: vi.fn<Backend['getJournal']>(() => Promise.resolve(structuredClone(journal))),
    onJournal: vi.fn((l: (e: JournalEvent) => void) => {
      journalListener = l
      return Promise.resolve(() => {
        journalListener = null
      })
    }),
    restartForUpdate: vi.fn<Backend['restartForUpdate']>(() => Promise.resolve()),
    exportDiagnostic: vi.fn<Backend['exportDiagnostic']>(() =>
      Promise.resolve<ExportOutcome>({ status: 'saved', path: 'C:\\Users\\exemple\\diagnostic.zip' }),
    ),
  } satisfies Backend
  return {
    backend,
    emit: (next: AppSnapshot | null) => {
      listener?.(next)
    },
    emitJournal: (event: JournalEvent) => {
      journalListener?.(event)
    },
  }
}

export { createMockBackend }

/** `component` with its props, inside the `TooltipProvider` that `App.vue` puts around every tab. */
export function inTooltips(component: Component, props: Record<string, unknown>): Component {
  return defineComponent({
    setup: () => () => h(TooltipProvider, null, { default: () => h(component, props) }),
  })
}
