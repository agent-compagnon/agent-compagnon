import { afterEach, describe, expect, it, vi } from 'vitest'
import { InvalidJournal } from '@/api/backend'
import { LIVE_PERIOD_MS, createMockBackend } from '@/api/mock'
import { eventEntry, exampleEntries } from '@/api/mock-journal'
import type { JournalEntry } from '@/api/types'
import { JOURNAL_LIMIT } from '@/components/journal/model'
import { NOW, snapshotOf, spyBackend } from '@/test/fixtures'
import { createJournal } from './journal'

function event(seq: number): JournalEntry {
  return eventEntry(seq, NOW + seq, `événement ${String(seq)}`)
}

function seqs(entries: readonly JournalEntry[]): number[] {
  return entries.map((e) => e.seq)
}

async function started(initial: JournalEntry[] = [], limit?: number) {
  const spy = spyBackend(snapshotOf('sessions'), initial)
  const journal = createJournal(spy.backend, limit)
  await journal.start()
  return { ...spy, journal }
}

afterEach(() => {
  vi.useRealTimers()
})

describe('journal of the window', () => {
  // Exigence : EX-018-01
  it('listens before it reads, then adds each new entry once', async () => {
    const { backend, emitJournal, journal } = await started(exampleEntries())
    expect(backend.onJournal).toHaveBeenCalledBefore(backend.getJournal)
    expect(journal.state.phase).toBe('ready')
    expect(seqs(journal.state.shown)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
    emitJournal({ type: 'entry', entry: event(11) })
    emitJournal({ type: 'entry', entry: event(11) })
    emitJournal({ type: 'entry', entry: event(12) })
    expect(seqs(journal.state.shown).slice(-3)).toEqual([10, 11, 12])
    journal.close()
    emitJournal({ type: 'entry', entry: event(13) })
    expect(journal.state.shown).toHaveLength(12)
  })

  // Exigence : EX-018-01
  it('in pause, keeps the list still and counts the entries that arrive; resuming shows them', async () => {
    const { emitJournal, journal } = await started([event(1), event(2)])
    journal.pause()
    const still = journal.state.shown
    emitJournal({ type: 'entry', entry: event(3) })
    emitJournal({ type: 'entry', entry: event(4) })
    expect(journal.state.shown).toBe(still)
    expect(seqs(journal.state.pending)).toEqual([3, 4])
    journal.resume()
    expect(journal.state.paused).toBe(false)
    expect(seqs(journal.state.shown)).toEqual([1, 2, 3, 4])
    expect(journal.state.pending).toEqual([])
  })

  // Exigence : EX-018-01
  it('receives the calls of the simulated backend as they come', async () => {
    vi.useFakeTimers()
    const journal = createJournal(createMockBackend('journal-direct', () => NOW))
    await journal.start()
    const before = journal.state.shown.length
    vi.advanceTimersByTime(LIVE_PERIOD_MS * 3)
    expect(journal.state.shown.length).toBe(before + 3)
    journal.pause()
    vi.advanceTimersByTime(LIVE_PERIOD_MS * 2)
    expect(journal.state.shown.length).toBe(before + 3)
    expect(journal.state.pending).toHaveLength(2)
    journal.resume()
    expect(journal.state.shown.length).toBe(before + 5)
    journal.close()
  })

  // Exigence : EX-018-05
  it('keeps at most its limit: the oldest go, pause or not', async () => {
    expect(JOURNAL_LIMIT).toBe(5000)
    const initial = Array.from({ length: 15 }, (_, i) => event(i + 1))
    const { emitJournal, journal } = await started(initial, 10)
    expect(seqs(journal.state.shown)).toEqual([6, 7, 8, 9, 10, 11, 12, 13, 14, 15])

    journal.pause()
    for (let seq = 16; seq <= 19; seq += 1) emitJournal({ type: 'entry', entry: event(seq) })
    // The entries waiting count: shown and waiting together stay within the limit.
    expect(journal.state.shown.length + journal.state.pending.length).toBe(10)
    expect(seqs(journal.state.shown)).toEqual([10, 11, 12, 13, 14, 15])
    expect(seqs(journal.state.pending)).toEqual([16, 17, 18, 19])
    for (let seq = 20; seq <= 30; seq += 1) emitJournal({ type: 'entry', entry: event(seq) })
    expect(journal.state.shown).toEqual([])
    expect(seqs(journal.state.pending)).toEqual([21, 22, 23, 24, 25, 26, 27, 28, 29, 30])
    journal.resume()
    expect(seqs(journal.state.shown)).toEqual([21, 22, 23, 24, 25, 26, 27, 28, 29, 30])
  })

  // Exigence : EX-018-05
  it('holds the large journal of the simulated backend within the limit', async () => {
    const journal = createJournal(createMockBackend('journal-volumineux', () => NOW))
    await journal.start()
    expect(journal.state.shown).toHaveLength(JOURNAL_LIMIT)
    const last = journal.state.shown[JOURNAL_LIMIT - 1]?.seq ?? 0
    expect(last).toBe(JOURNAL_LIMIT)
  })

  // Exigence : EX-018-12
  it('asks the whole journal again after a resynchronisation, and merges it without duplicate', async () => {
    const { backend, emitJournal, journal } = await started([event(1), event(2)], 5)
    emitJournal({ type: 'entry', entry: event(3) })
    // Entries 4 and 5 were lost on the way; the application still holds 1 to 6.
    backend.getJournal.mockResolvedValueOnce([1, 2, 3, 4, 5, 6].map(event))
    emitJournal({ type: 'resync' })
    await vi.waitFor(() => {
      expect(seqs(journal.state.shown)).toEqual([2, 3, 4, 5, 6])
    })
    expect(backend.getJournal).toHaveBeenCalledTimes(2)

    // Dropped by the limit, an entry does not come back; in pause, the new ones wait.
    journal.pause()
    backend.getJournal.mockResolvedValueOnce([1, 2, 3, 4, 5, 6, 7].map(event))
    emitJournal({ type: 'resync' })
    await vi.waitFor(() => {
      expect(seqs(journal.state.pending)).toEqual([7])
    })
    expect(seqs(journal.state.shown)).toEqual([3, 4, 5, 6])
  })

  it('says when the journal cannot be read, and shows nothing of an unreadable one', async () => {
    const { backend, journal } = await started()
    backend.getJournal.mockRejectedValueOnce('filtre de sortie indisponible : journal non transmis')
    await journal.resync()
    expect(journal.state.phase).toBe('unavailable')
    backend.getJournal.mockRejectedValueOnce(new InvalidJournal())
    await journal.resync()
    expect(journal.state.phase).toBe('invalid')
  })

  // Exigence : EX-018-04
  it('exports a diagnostic: the path on success, the reason on failure, nothing when the dialog is closed', async () => {
    const { backend, journal } = await started()
    await journal.exportDiagnostic()
    expect(backend.exportDiagnostic).toHaveBeenCalledTimes(1)
    expect(journal.state.export).toEqual({ phase: 'saved', path: 'C:\\Users\\exemple\\diagnostic.zip' })

    backend.exportDiagnostic.mockResolvedValueOnce({ status: 'cancelled' })
    await journal.exportDiagnostic()
    expect(journal.state.export).toEqual({ phase: 'saved', path: 'C:\\Users\\exemple\\diagnostic.zip' })
    journal.dismissExport()
    backend.exportDiagnostic.mockResolvedValueOnce({ status: 'cancelled' })
    await journal.exportDiagnostic()
    expect(journal.state.export).toEqual({ phase: 'idle' })

    backend.exportDiagnostic.mockRejectedValueOnce('export de diagnostic impossible : accès refusé')
    await journal.exportDiagnostic()
    expect(journal.state.export).toEqual({
      phase: 'failed',
      reason: 'export de diagnostic impossible : accès refusé',
    })

    // One export at a time: the dialog is already open.
    let answer: () => void = () => undefined
    backend.exportDiagnostic.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = () => {
            resolve({ status: 'saved', path: 'D:\\diag.zip' })
          }
        }),
    )
    const first = journal.exportDiagnostic()
    expect(journal.state.export).toEqual({ phase: 'running' })
    await journal.exportDiagnostic()
    answer()
    await first
    expect(backend.exportDiagnostic).toHaveBeenCalledTimes(5)
    expect(journal.state.export).toEqual({ phase: 'saved', path: 'D:\\diag.zip' })
  })

  // Exigence : EX-018-04
  it('the simulated export succeeds or fails by scenario', async () => {
    vi.useFakeTimers()
    const saved = createMockBackend('sessions', () => NOW).exportDiagnostic()
    const failed = createMockBackend('export-echec', () => NOW).exportDiagnostic()
    const cancelled = createMockBackend('export-annule', () => NOW).exportDiagnostic()
    vi.advanceTimersByTime(1000)
    await expect(saved).resolves.toEqual({
      status: 'saved',
      path: 'C:\\Users\\exemple\\Downloads\\mcp-proxy-diagnostic-20261122-1400.zip',
    })
    await expect(failed).rejects.toThrow('export de diagnostic impossible')
    await expect(cancelled).resolves.toEqual({ status: 'cancelled' })
  })
})
