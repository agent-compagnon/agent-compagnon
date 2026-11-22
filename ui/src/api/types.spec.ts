import { describe, expect, it } from 'vitest'
import journalExample from './journal.example.json'
import example from './snapshot.example.json'
import { parseEntry, parseExportOutcome, parseJournal, parseSnapshot } from './types'

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

  // Exigence : EX-020-17
  it('reads the update put in place, and nothing else in its place', () => {
    expect(parseSnapshot(structuredClone(example))?.update).toEqual({ version: '0.2.0', critical: true })
    expect(parseSnapshot({ ...structuredClone(example), update: null })?.update).toBeNull()
    expect(parseSnapshot({ ...structuredClone(example), update: '0.2.0' })).toBeNull()
    expect(parseSnapshot({ ...structuredClone(example), update: { version: '0.2.0' } })).toBeNull()
    expect(
      parseSnapshot({ ...structuredClone(example), update: { version: '0.2.0', critical: false, url: 'x' } }),
    ).toBeNull()
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

describe('contract of the journal', () => {
  // Exigence : EX-018-10
  it('reads exactly the journal and the export answers written by the gateway', () => {
    const entries = parseJournal(structuredClone(journalExample.getJournal))
    expect(entries?.map((e) => e.seq)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
    expect(entries?.map((e) => e.kind)).toEqual([
      'call',
      'call',
      'call',
      'call',
      'call',
      'call',
      'call',
      'toolsChanged',
      'event',
      'panic',
    ])
    // Every optional field of the fixture is filled at least once (see `journal_contract.rs`).
    expect(entries?.some((e) => e.details?.policy !== null && e.details?.session !== null)).toBe(true)
    expect(entries?.some((e) => e.details !== null && e.details.session === null)).toBe(true)
    expect(entries?.some((e) => (e.details?.masked.kinds.length ?? 0) > 0)).toBe(true)
    expect(entries?.some((e) => e.slow)).toBe(true)
    expect(journalExample.exportDiagnostic.map((o) => parseExportOutcome(o)?.status)).toEqual(['saved', 'cancelled'])
  })

  // Exigence : EX-018-10
  it('refuses an entry with a key more, a key less or a value of another type', () => {
    const [first] = structuredClone(journalExample.getJournal.entries)
    expect(parseEntry(first)).not.toBeNull()
    expect(parseEntry({ ...first, secret: 'x' })).toBeNull()
    expect(parseEntry({ ...first, status: 'exploded' })).toBeNull()
    expect(parseEntry({ ...first, seq: '1' })).toBeNull()
    const missing: Record<string, unknown> = { ...first }
    delete missing.problem
    expect(parseEntry(missing)).toBeNull()
    const error = structuredClone(journalExample.getJournal.entries[1])
    Object.assign(error?.details ?? {}, { token: 'x' })
    expect(parseJournal({ entries: [error] })).toBeNull()
    expect(parseJournal({ entries: [first], more: true })).toBeNull()
    expect(parseExportOutcome({ status: 'saved' })).toBeNull()
    expect(parseExportOutcome({ status: 'cancelled', path: 'C:\\x.zip' })).toBeNull()
  })
})
