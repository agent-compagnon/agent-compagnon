import { describe, expect, it } from 'vitest'
import { terms } from '@/lib/search'
import { snapshotOf } from '@/test/fixtures'
import {
  environmentChoices,
  filterRows,
  matchRow,
  overThreshold,
  rows,
  switchBlocked,
  updateBanners,
} from './model'

const all = { environment: null, focus: null, query: '' }
const ids = (query: string, environment: string | null = null) =>
  filterRows(rows(snapshotOf('sessions')), { ...all, query, environment }).map((v) => v.row.id)
const row = (id: string, scenario: Parameters<typeof snapshotOf>[0] = 'sessions') => {
  const found = rows(snapshotOf(scenario)).find((r) => r.id === id)
  if (!found) throw new Error(id)
  return found
}

describe('MCP tab model', () => {
  // Exigence : EX-017-02
  it('searches every field of a row', () => {
    expect(ids('kibana')).toHaveLength(8) // MCP label
    expect(ids('base de donnees')).toHaveLength(4) // label, accents aside
    expect(ids('local')).toEqual(['rec/jira', 'prod/jira']) // kind
    expect(ids('pra').every((id) => id.startsWith('pra/') || id.startsWith('pprod/') || id.startsWith('prod/'))).toBe(
      true,
    ) // environment
    expect(ids('sso entreprise')).toEqual(ids('gitlab').concat(ids('base de donnees'))) // session
    expect(ids('injoignable')).toEqual(['prod/kibana-logs']) // state
    expect(ids('refuse').length).toBeGreaterThan(0) // policy, accents aside
    expect(ids('notification')).toEqual(ids('refusé'))
    expect(matchRow(row('dev/kibana-logs'), terms('kibana'))?.tools).toBeNull()
  })

  // Exigence : EX-017-02
  it('unfolds a row whose tools match, with those tools only; every word must match', () => {
    const byDescription = matchRow(row('dev/kibana-logs'), terms('lucene'))
    expect(byDescription?.tools?.map((t) => t.name)).toEqual(['search_logs'])
    const byName = matchRow(row('dev/kibana-logs'), terms('DELETE_INDEX'))
    expect(byName?.tools?.map((t) => t.name)).toEqual(['delete_index'])
    // One word on the row, the other on a tool.
    expect(ids('prod index')).toEqual(['pprod/kibana-logs', 'prod/kibana-logs', 'pprod/db-readonly', 'prod/db-readonly'])
    // Both words must be found in one same tool.
    expect(matchRow(row('dev/kibana-logs'), terms('lucene delete'))).toBeNull()
    expect(ids('kibana grafana')).toEqual([])
  })

  // Exigence : EX-017-03
  it('filters on one environment, exposed or not, with exact counts', () => {
    const snapshot = snapshotOf('sessions')
    expect(snapshot.environments.find((e) => e.id === 'prod')?.exposed).toBe(false)
    expect(ids('', 'prod')).toEqual([
      'prod/kibana-logs',
      'prod/gitlab-ops',
      'prod/k8s-admin',
      'prod/db-readonly',
      'prod/jira',
    ])
    const counts = Object.fromEntries(environmentChoices(snapshot).map((c) => [c.id ?? 'all', c.count]))
    expect(counts).toEqual({ all: 24, dev: 3, int: 3, qua: 1, rec: 5, perf: 2, pprod: 3, prod: 5, pra: 2 })
    expect(ids('kibana', 'prod')).toEqual(['prod/kibana-logs'])
  })

  // Exigence : EX-017-04
  it('says why a switch cannot be used', () => {
    const snapshot = snapshotOf('sessions')
    expect(switchBlocked(snapshot, row('dev/kibana-logs'))).toBeNull()
    expect(switchBlocked(snapshot, row('prod/kibana-logs'))).toBe(
      "PROD n'est pas exposé : exposez-le d'abord avec sa pastille.",
    )
    const first = snapshotOf('premier-lancement')
    expect(switchBlocked(first, row('dev/kibana-logs', 'premier-lancement'))).toBe("Choisissez d'abord un profil.")
  })

  // Exigence : EX-017-05
  it('words the update banner', () => {
    expect(updateBanners(snapshotOf('mcp-mis-a-jour')).map((b) => b.text)).toEqual([
      'Kibana Logs (REC) a été mis à jour : 2 nouveaux outils et 1 outil modifié.',
      'GitLab Ops (DEV) a été mis à jour : 1 nouvel outil et 1 outil retiré.',
    ])
    expect(updateBanners(snapshotOf('sessions'))).toEqual([])
  })

  // Exigence : EX-017-06
  it('is over the threshold strictly above it', () => {
    const snapshot = snapshotOf('sessions')
    expect(overThreshold(snapshot)).toBe(false)
    expect(overThreshold({ ...snapshot, exposedTools: 100, exposedToolsThreshold: 100 })).toBe(false)
    expect(overThreshold({ ...snapshot, exposedTools: 101, exposedToolsThreshold: 100 })).toBe(true)
    const over = snapshotOf('seuil')
    expect(over.exposedTools).toBe(106)
    expect(overThreshold(over)).toBe(true)
    expect(over.alerts.map((a) => a.kind)).toEqual(['toolThreshold'])
  })
})
