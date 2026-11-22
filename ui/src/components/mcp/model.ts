// What the MCP tab shows, computed from the snapshot alone: one row per instance (an MCP in an environment), the
// environment filter and its counts, the full-text search, the update banners. Pure: the components only render.

import type {
  AppSnapshot,
  Category,
  DecisionKind,
  EnvironmentView,
  InstanceView,
  McpView,
  PolicyView,
  SessionView,
  ToolView,
} from '@/api/types'
import { fr } from '@/i18n/fr'
import { anyContains, terms } from '@/lib/search'

export const CATEGORIES: readonly Category[] = ['read', 'write', 'destructive']

export interface McpRow {
  /** `env/mcp`, stable across snapshots. */
  id: string
  mcp: McpView
  instance: InstanceView
  environment: EnvironmentView
  /** Order of the environment in the configuration, for sorting. */
  environmentIndex: number
  session: SessionView | null
  /** Tools added or modified since the user last acknowledged the update of this instance. */
  added: ReadonlySet<string>
  modified: ReadonlySet<string>
}

export function rowId(environment: string, mcp: string): string {
  return `${environment}/${mcp}`
}

/** Every instance, in the order of the configuration: MCPs, then environments. */
export function rows(snapshot: AppSnapshot): McpRow[] {
  const out: McpRow[] = []
  for (const mcp of snapshot.mcps) {
    for (const instance of mcp.instances) {
      const environmentIndex = snapshot.environments.findIndex((e) => e.id === instance.environment)
      const environment = snapshot.environments[environmentIndex]
      if (environment === undefined) continue
      out.push({
        id: rowId(instance.environment, mcp.id),
        mcp,
        instance,
        environment,
        environmentIndex,
        session: snapshot.sessions.find((s) => s.id === instance.session) ?? null,
        added: new Set(instance.update?.added ?? []),
        modified: new Set(instance.update?.modified ?? []),
      })
    }
  }
  return out
}

/** « Kibana Logs (PROD) » */
export function rowName(row: McpRow): string {
  return fr.mcp.instanceName(row.mcp.label, row.environment.label)
}

/** Short word of a decision: « autorisé », « confirmation », « refusé ». */
export function decisionWord(decision: DecisionKind): string {
  return fr.mcp.decision[decision]
}

/** The words searched for the policy of a row, as the tooltip says them. */
function policyTexts(row: McpRow): string[] {
  return CATEGORIES.flatMap((c) => {
    const policy: PolicyView = row.instance.policy[c]
    return [`${fr.mcp.category[c]} ${decisionWord(policy.decision)}`, policy.label]
  })
}

/** What the search reads in a row, outside of its tools. */
export function rowTexts(row: McpRow): string[] {
  return [
    row.mcp.label,
    row.mcp.id,
    row.mcp.local ? fr.mcp.local : fr.mcp.remote,
    row.environment.label,
    row.environment.id,
    row.session?.label ?? '',
    fr.mcp.state[row.instance.state],
    ...policyTexts(row),
  ]
}

/** What the search reads in a tool: its names and its description. */
export function toolTexts(tool: ToolView): string[] {
  return [tool.name, tool.exposedName, tool.description]
}

export interface Match {
  /** `null`: the row matches by its own fields; otherwise the tools that match, which the row unfolds to show. */
  tools: ToolView[] | null
}

/**
 * Every word must match. A word found in the fields of the row is satisfied by it; the remaining words must all be
 * found in one same tool, and the row then shows those tools. `null` when the row does not match.
 */
export function matchRow(row: McpRow, words: readonly string[]): Match | null {
  if (words.length === 0) return { tools: null }
  const own = rowTexts(row)
  const rest = words.filter((w) => !anyContains(own, w))
  if (rest.length === 0) return { tools: null }
  const tools = row.instance.tools.filter((t) => {
    const texts = toolTexts(t)
    return rest.every((w) => anyContains(texts, w))
  })
  return tools.length > 0 ? { tools } : null
}

export interface Filters {
  /** `null`: every environment. Independent of the exposure. */
  environment: string | null
  /** « Voir » of an update banner: this row only. */
  focus: string | null
  query: string
}

export interface Visible {
  row: McpRow
  match: Match
}

export function filterRows(all: readonly McpRow[], filters: Filters): Visible[] {
  const words = terms(filters.query)
  const out: Visible[] = []
  for (const row of all) {
    if (filters.focus !== null && row.id !== filters.focus) continue
    if (filters.environment !== null && row.environment.id !== filters.environment) continue
    const match = matchRow(row, words)
    if (match) out.push({ row, match })
  }
  return out
}

export interface EnvironmentChoice {
  /** `null` for « Tous ». */
  id: string | null
  label: string
  /** Number of MCPs, whatever the exposure. */
  count: number
}

export function environmentChoices(snapshot: AppSnapshot): EnvironmentChoice[] {
  const all = rows(snapshot)
  return [
    { id: null, label: fr.mcp.filter.all, count: all.length },
    ...snapshot.environments.map((e) => ({
      id: e.id,
      label: e.label,
      count: all.filter((r) => r.environment.id === e.id).length,
    })),
  ]
}

/** The tool counter turns orange strictly above the threshold, as `Registry::over_threshold`. */
export function overThreshold(snapshot: AppSnapshot): boolean {
  return snapshot.exposedTools > snapshot.exposedToolsThreshold
}

export interface UpdateBanner {
  row: McpRow
  text: string
}

export function updateBanners(snapshot: AppSnapshot): UpdateBanner[] {
  return rows(snapshot)
    .filter((r) => r.instance.update !== null)
    .map((row) => {
      const update = row.instance.update
      return {
        row,
        text: fr.mcp.update.text(
          rowName(row),
          update?.added.length ?? 0,
          update?.modified.length ?? 0,
          update?.removed.length ?? 0,
        ),
      }
    })
}

/** Why the switch of a row cannot be used; `null` when it can. */
export function switchBlocked(snapshot: AppSnapshot, row: McpRow): string | null {
  if (snapshot.profile.chosen === null) return fr.mcp.blocked.noProfile
  if (!row.environment.exposed) return fr.mcp.blocked.environment(row.environment.label)
  return null
}

/** Severity of a decision, for sorting by policy. */
const SEVERITY: Record<DecisionKind, number> = { allow: 0, confirmElicitation: 1, confirmNotification: 2, deny: 3 }

export function policyRank(row: McpRow): number {
  return CATEGORIES.reduce((sum, c) => sum * 4 + SEVERITY[row.instance.policy[c].decision], 0)
}

const STATE_ORDER = ['connected', 'catalogue', 'sessionRequired', 'unreachable', 'disabled'] as const

export function stateRank(row: McpRow): number {
  return STATE_ORDER.indexOf(row.instance.state)
}
