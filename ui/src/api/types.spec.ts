import { describe, expect, it } from 'vitest'
import example from './snapshot.example.json'
import { parseSnapshot } from './types'

describe('contract of the snapshot', () => {
  // Exigence : EX-016-06
  it('reads exactly the snapshot of the example configuration written by the gateway', () => {
    const snapshot = parseSnapshot(structuredClone(example))
    expect(snapshot).not.toBeNull()
    expect(snapshot?.sessions.map((s) => s.id)).toContain('sso-corp')
    // Every optional field of the fixture is filled at least once (see `snapshot_contract.rs`).
    expect(snapshot?.sessions.some((s) => s.state.requestedBy === 'Claude Desktop')).toBe(true)
    expect(snapshot?.sessions.some((s) => s.state.at !== null)).toBe(true)
    expect(snapshot?.mcps.some((m) => m.instances.some((i) => i.update !== null))).toBe(true)
  })

  // Exigence : EX-016-06
  it('refuses a snapshot with a key more, a key less or a value of another type', () => {
    const extra = { ...structuredClone(example), password: 'x' }
    expect(parseSnapshot(extra)).toBeNull()
    const nested = structuredClone(example)
    Object.assign(nested.sessions[0]?.state ?? {}, { token: 'x' })
    expect(parseSnapshot(nested)).toBeNull()
    const missing: Record<string, unknown> = structuredClone(example)
    delete missing.gatewaySuspended
    expect(parseSnapshot(missing)).toBeNull()
    const kind = structuredClone(example)
    Object.assign(kind.sessions[0]?.state ?? {}, { kind: 'connected' })
    expect(parseSnapshot(kind)).toBeNull()
    expect(parseSnapshot(null)).toBeNull()
  })
})
