import { describe, expect, it } from 'vitest'
import { contains, highlight, normalize, terms } from './search'

describe('search normalization', () => {
  // Exigence : EX-017-01
  it('ignores accents and case', () => {
    expect(contains('Developpeur', normalize('développeur'))).toBe(true)
    expect(contains('prod', normalize('PROD'))).toBe(true)
    expect(contains('Kibana', normalize('kibána'))).toBe(true)
    expect(contains('Kibána Logs', normalize('KIBANA'))).toBe(true)
    expect(normalize('Écriture DÉSTRUCTIVE')).toBe('ecriture destructive')
    expect(contains('Kibana', normalize('grafana'))).toBe(false)
  })

  // Exigence : EX-017-01
  it('splits a query in normalized words', () => {
    expect(terms('  Kibána   INDEX kibana ')).toEqual(['kibana', 'index'])
    expect(terms('   ')).toEqual([])
  })

  // Exigence : EX-017-02
  it('highlights the original characters, and returns text only', () => {
    expect(highlight('Kibána Logs', ['kibana'])).toEqual([
      { text: 'Kibána', match: true },
      { text: ' Logs', match: false },
    ])
    expect(highlight('delete_index', ['index', 'del'])).toEqual([
      { text: 'del', match: true },
      { text: 'ete_', match: false },
      { text: 'index', match: true },
    ])
    // Markup in a server description stays text in the segments.
    expect(highlight('<img src=x onerror=alert(1)> index', ['index'])).toEqual([
      { text: '<img src=x onerror=alert(1)> ', match: false },
      { text: 'index', match: true },
    ])
    expect(highlight('', ['x'])).toEqual([])
    expect(highlight('🦀 crab é', ['e'])).toEqual([
      { text: '🦀 crab ', match: false },
      { text: 'é', match: true },
    ])
  })
})
