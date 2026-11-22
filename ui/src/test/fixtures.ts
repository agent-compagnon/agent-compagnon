// Snapshots for the tests of the window: the scenarios of the simulated backend, at a fixed time.
import { type Scenario, createMockBackend, scenario } from '@/api/mock'
import type { Backend } from '@/api/backend'
import type { AppSnapshot } from '@/api/types'
import { vi } from 'vitest'

/** Sun, 22 Nov 2026 14:00 UTC. */
export const NOW = Date.UTC(2026, 10, 22, 14, 0)

export function snapshotOf(name: Scenario): AppSnapshot {
  return scenario(name, NOW)
}

/** A backend whose commands are spies, answering with `snapshot`. */
export function spyBackend(snapshot: AppSnapshot) {
  let listener: ((s: AppSnapshot | null) => void) | null = null
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
  } satisfies Backend
  return {
    backend,
    emit: (next: AppSnapshot | null) => {
      listener?.(next)
    },
  }
}

export { createMockBackend }
