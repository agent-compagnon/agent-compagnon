import { describe, expect, it } from 'vitest'
import { exampleEntries } from '@/api/mock-journal'
import { terms } from '@/lib/search'
import { snapshotOf } from '@/test/fixtures'
import {
  type Criteria,
  type StatusFilter,
  environmentChoices,
  formatDuration,
  formatTime,
  isError,
  labelsOf,
  matches,
  merge,
  rowOf,
} from './model'

// The entries of `journal.example.json`, written by the gateway: 1 ok (DEV, Kibana Logs), 2 error (REC, GitLab Ops),
// 3 slow (DEV, Kibana Logs), 4 denied (PROD, Kubernetes Admin), 5 leak masked (REC, Kibana Logs), 6 confirmed (PROD,
// GitLab Ops), 7 blocked (REC, Jira), 8 tool list change (DEV, GitLab Ops), 9 event, 10 panic.
const entries = exampleEntries()
const labels = labelsOf(snapshotOf('sessions'))
const rows = entries.map((e) => rowOf(e, labels))

function select(criteria: Partial<Criteria> & { query?: string }): number[] {
  const full: Criteria = {
    environment: criteria.environment ?? null,
    status: criteria.status ?? 'all',
    words: terms(criteria.query ?? ''),
  }
  return rows.filter((r) => matches(r, full)).map((r) => r.entry.seq)
}

describe('rows of the journal', () => {
  // Exigence : EX-018-02
  it('filters by status as the core classified, by environment, and both', () => {
    const status = (s: StatusFilter) => select({ status: s })
    expect(status('all')).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
    expect(status('problems')).toEqual([2, 3, 5, 7, 10])
    expect(status('policies')).toEqual([4, 6, 7])
    expect(select({ environment: 'dev' })).toEqual([1, 3, 8])
    expect(select({ environment: 'rec' })).toEqual([2, 5, 7])
    expect(select({ environment: 'rec', status: 'problems' })).toEqual([2, 5, 7])
    expect(select({ environment: 'prod', status: 'policies' })).toEqual([4, 6])
    expect(select({ environment: 'prod', status: 'problems' })).toEqual([])
    expect(select({ environment: 'dev', status: 'problems', query: 'kibana' })).toEqual([3])
  })

  // Exigence : EX-018-02
  it('searches the client, the tool, the MCP and the status, accents and case aside', () => {
    expect(select({ query: 'CLAUDE desktop' })).toEqual([1, 2, 3, 4, 5, 6, 7])
    expect(select({ query: 'cancel_pipeline' })).toEqual([6])
    expect(select({ query: 'kibána' })).toEqual([1, 3, 5])
    expect(select({ query: 'k8s-admin' })).toEqual([4])
    expect(select({ query: 'kubernetes' })).toEqual([4])
    expect(select({ query: 'INTERDIT' })).toEqual([4])
    expect(select({ query: 'fuite masquee' })).toEqual([5])
    expect(select({ query: 'lent' })).toEqual([3])
    expect(select({ query: 'réussi dev' })).toEqual([1, 3])
    // Environments by label or identifier; events by their text.
    expect(select({ query: 'PROD jira' })).toEqual([])
    expect(select({ query: 'exposé' })).toEqual([9])
    expect(select({ query: 'panic' })).toEqual([10])
    expect(select({ query: 'aucune-correspondance' })).toEqual([])
  })

  // Exigence : EX-018-02
  it('counts the entries of each environment, whatever the filters', () => {
    const choices = environmentChoices(entries, snapshotOf('sessions'))
    expect(choices[0]).toEqual({ id: null, label: 'Tous', count: 10 })
    expect(choices.find((c) => c.id === 'dev')?.count).toBe(3)
    expect(choices.find((c) => c.id === 'rec')?.count).toBe(3)
    expect(choices.find((c) => c.id === 'prod')?.count).toBe(2)
    expect(choices.find((c) => c.id === 'pra')?.count).toBe(0)
  })

  // Exigence : EX-018-03
  it('unfolds only the calls whose content the core kept', () => {
    const expandable = rows.filter((r) => r.expandable).map((r) => r.entry.seq)
    expect(expandable).toEqual([2, 3, 4, 5, 7])
  })

  it('shows in red what went wrong, never a refusal of the policy', () => {
    expect(rows.filter((r) => r.error).map((r) => r.entry.seq)).toEqual([2, 5, 10])
    expect(entries.filter(isError).map((e) => e.status)).not.toContain('denied')
    expect(rows[3]?.status).toBe('interdit')
    expect(rows[8]?.status).toBe('événement')
    expect(rows[7]?.status).toBe("liste d'outils")
    expect(rows[3]?.mcp).toBe('Kubernetes Admin')
    expect(rows[3]?.environment).toBe('PROD')
  })

  // Exigence : EX-018-12
  it('merges by seq, once each, oldest first', () => {
    const [a, b, c] = entries
    if (!a || !b || !c) throw new Error('fixture')
    expect(merge([c, a], [b, a, c]).map((e) => e.seq)).toEqual([1, 2, 3])
  })

  it('formats times in local time and durations in French', () => {
    const at = Date.UTC(2026, 10, 22, 13, 3, 7)
    const now = Date.UTC(2026, 10, 22, 14, 0)
    const local = new Date(at)
    const hh = String(local.getHours()).padStart(2, '0')
    expect(formatTime(at, now)).toBe(`${hh}:03:07`)
    expect(formatTime(at, now + 86_400_000)).toMatch(new RegExp(`^\\d{2}/11 ${hh}:03:07$`))
    expect(formatDuration(182)).toBe('182 ms')
    expect(formatDuration(2310)).toBe('2,3 s')
    expect(formatDuration(65_000)).toBe('1 min 05 s')
  })
})
