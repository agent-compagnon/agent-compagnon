// What the Journal tab shows, computed from the entries the core sent and the snapshot (labels of the environments
// and MCP). Pure functions: no DOM, no HTML. The classification of an entry (problem, policy) is the core's; the
// window only reads it.

import type { AppSnapshot, JournalEntry } from '@/api/types'
import { fr } from '@/i18n/fr'
import { normalize } from '@/lib/search'

/** Entries the window keeps at most, shown and waiting during a pause together; the oldest go first. */
export const JOURNAL_LIMIT = 5000

export type StatusFilter = 'all' | 'problems' | 'policies'

/** `known` and `incoming` together, each `seq` once, oldest first. */
export function merge(known: readonly JournalEntry[], incoming: readonly JournalEntry[]): JournalEntry[] {
  const bySeq = new Map<number, JournalEntry>()
  for (const entry of known) bySeq.set(entry.seq, entry)
  for (const entry of incoming) if (!bySeq.has(entry.seq)) bySeq.set(entry.seq, entry)
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq)
}

/** Red for what went wrong; a refusal of the policy protects, it is not an error (black and white). */
export function isError(entry: JournalEntry): boolean {
  return entry.kind === 'panic' || entry.status === 'error' || entry.status === 'leakMasked'
}

export interface Labels {
  environments: ReadonlyMap<string, string>
  mcps: ReadonlyMap<string, string>
}

export function labelsOf(snapshot: AppSnapshot): Labels {
  return {
    environments: new Map(snapshot.environments.map((e) => [e.id, e.label])),
    mcps: new Map(snapshot.mcps.map((m) => [m.id, m.label])),
  }
}

export interface JournalRow {
  entry: JournalEntry
  environment: string | null
  mcp: string | null
  /** What the status column says: the label of the core for a call, the kind of the entry otherwise. */
  status: string
  error: boolean
  /** Only a call whose content the core kept unfolds. */
  expandable: boolean
  /** Normalized texts the search looks into. */
  fields: string[]
}

export function rowOf(entry: JournalEntry, labels: Labels): JournalRow {
  const environment = entry.environment === null ? null : (labels.environments.get(entry.environment) ?? entry.environment)
  const mcp = entry.mcp === null ? null : (labels.mcps.get(entry.mcp) ?? entry.mcp)
  const status = entry.statusLabel ?? fr.journal.kind[entry.kind]
  const texts = [
    entry.client,
    entry.tool,
    environment,
    entry.environment,
    mcp,
    entry.mcp,
    status,
    entry.status,
    entry.slow ? fr.journal.slow : null,
    entry.summary,
  ]
  return {
    entry,
    environment,
    mcp,
    status,
    error: isError(entry),
    expandable: entry.details !== null,
    fields: texts.filter((t): t is string => t !== null && t.length > 0).map(normalize),
  }
}

export interface Criteria {
  /** An environment identifier, `null` for all. */
  environment: string | null
  status: StatusFilter
  /** Normalized words (`terms`): each must be found in one field of the row. */
  words: readonly string[]
}

export function matches(row: JournalRow, criteria: Criteria): boolean {
  const entry = row.entry
  if (criteria.environment !== null && entry.environment !== criteria.environment) return false
  if (criteria.status === 'problems' && !entry.problem) return false
  if (criteria.status === 'policies' && !entry.policy) return false
  return criteria.words.every((word) => row.fields.some((field) => field.includes(word)))
}

export interface EnvironmentChoice {
  /** `null`: every environment. */
  id: string | null
  label: string
  count: number
}

/** « Tous », then each environment of the snapshot, with the number of entries held for it, whatever the filters. */
export function environmentChoices(entries: readonly JournalEntry[], snapshot: AppSnapshot): EnvironmentChoice[] {
  const counts = new Map<string, number>()
  for (const entry of entries) {
    if (entry.environment !== null) counts.set(entry.environment, (counts.get(entry.environment) ?? 0) + 1)
  }
  return [
    { id: null, label: fr.journal.filter.all, count: entries.length },
    ...snapshot.environments.map((e) => ({ id: e.id, label: e.label, count: counts.get(e.id) ?? 0 })),
  ]
}

const time = new Intl.DateTimeFormat('fr-FR', { hour: '2-digit', minute: '2-digit', second: '2-digit' })
const day = new Intl.DateTimeFormat('fr-FR', { day: '2-digit', month: '2-digit' })

/** Local time: `14:03:07`, with the day before today (`21/11 14:03:07`). */
export function formatTime(at: number, now: number): string {
  const sameDay = new Date(at).toDateString() === new Date(now).toDateString()
  return sameDay ? time.format(at) : `${day.format(at)} ${time.format(at)}`
}

const seconds = new Intl.NumberFormat('fr-FR', { minimumFractionDigits: 1, maximumFractionDigits: 1 })

/** `182 ms`, `2,3 s`, `1 min 05 s`. */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${String(ms)} ms`
  if (ms < 60_000) return `${seconds.format(ms / 1000)} s`
  const total = Math.round(ms / 1000)
  return `${String(Math.floor(total / 60))} min ${String(total % 60).padStart(2, '0')} s`
}

/** Request or response, indented: shown as text, never interpreted. */
export function prettyJson(value: unknown): string {
  if (value === undefined) return ''
  return JSON.stringify(value, null, 2)
}
