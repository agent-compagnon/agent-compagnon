// Journal of the simulated backend (`--mode mock` only, through `mock.ts`): entries shaped like those of the core,
// generated from the example configuration. Deterministic: the same index gives the same entry. The classification
// mirrors the core (`is_problem`, `is_policy`, `keeps_content`) because this stands for the core; the window itself
// never computes it. No value here looks like a real secret.

import example from './journal.example.json'
import snapshotExample from './snapshot.example.json'
import {
  type AppSnapshot,
  type CallStatus,
  type DecisionKind,
  type JournalEntry,
  type PolicyView,
  parseJournal,
  parseSnapshot,
} from './types'

const LABELS: Record<CallStatus, string> = {
  ok: 'réussi',
  leakMasked: 'fuite masquée',
  error: 'erreur',
  denied: 'interdit',
  refused: 'refusé',
  timeout: 'délai dépassé',
  blocked: 'bloqué',
  rateLimited: 'limite de débit',
  authRequired: 'connexion requise',
  suspended: 'passerelle suspendue',
  inputRequired: 'confirmation demandée',
  cancelled: 'annulé',
}

const SLOW_MS = 5000

function keepsContent(status: CallStatus, slow: boolean): boolean {
  return slow || !['ok', 'inputRequired', 'suspended'].includes(status)
}

function isProblem(status: CallStatus, slow: boolean): boolean {
  return slow || ['error', 'leakMasked', 'blocked', 'timeout', 'rateLimited', 'authRequired', 'cancelled'].includes(status)
}

function isPolicy(status: CallStatus, decision: DecisionKind | null): boolean {
  return (
    decision === 'confirmElicitation' ||
    decision === 'confirmNotification' ||
    ['denied', 'refused', 'timeout', 'suspended', 'inputRequired'].includes(status)
  )
}

/** The entries of the contract file (`journal.example.json`), as the core wrote them. */
export function exampleEntries(): JournalEntry[] {
  const entries = parseJournal(structuredClone(example.getJournal))
  if (entries === null) throw new Error('journal.example.json does not match types.ts')
  return entries
}

interface Tool {
  environment: string
  mcp: string
  exposedName: string
  name: string
  session: string | null
  policy: PolicyView
}

let tools: Tool[] | null = null

/** The tools of the example configuration in the environments a working day touches. */
function toolsOf(): Tool[] {
  if (tools) return tools
  const snapshot: AppSnapshot | null = parseSnapshot(structuredClone(snapshotExample))
  if (snapshot === null) throw new Error('snapshot.example.json does not match types.ts')
  const environments = ['dev', 'int', 'rec', 'pprod', 'prod']
  tools = snapshot.mcps.flatMap((m) =>
    m.instances
      .filter((i) => environments.includes(i.environment))
      .flatMap((i) =>
        i.tools.map((t) => ({
          environment: i.environment,
          mcp: m.id,
          exposedName: t.exposedName,
          name: t.name,
          session: i.session,
          policy: t.policy,
        })),
      ),
  )
  return tools
}

/** A small deterministic generator: the same seed gives the same draws. */
function draws(seed: number) {
  let state = (seed * 2654435761) >>> 0 || 1
  return () => {
    state ^= state << 13
    state ^= state >>> 17
    state ^= state << 5
    state >>>= 0
    return state / 0x1_0000_0000
  }
}

const CLIENTS = ['Claude Desktop', 'Visual Studio Code', 'Claude Desktop', 'Claude Desktop']

function statusOf(draw: number, policy: PolicyView): { status: CallStatus; slow: boolean } {
  if (policy.decision === 'deny') return { status: 'denied', slow: false }
  if (policy.decision !== 'allow') {
    if (draw < 0.15) return { status: 'refused', slow: false }
    if (draw < 0.22) return { status: 'timeout', slow: false }
    if (draw < 0.3) return { status: 'inputRequired', slow: false }
    return { status: 'ok', slow: draw > 0.95 }
  }
  if (draw < 0.06) return { status: 'error', slow: false }
  if (draw < 0.08) return { status: 'blocked', slow: false }
  if (draw < 0.095) return { status: 'leakMasked', slow: false }
  if (draw < 0.11) return { status: 'authRequired', slow: false }
  if (draw < 0.12) return { status: 'rateLimited', slow: false }
  if (draw < 0.125) return { status: 'cancelled', slow: false }
  return { status: 'ok', slow: draw > 0.96 }
}

function responseOf(status: CallStatus, tool: Tool, count: number): unknown {
  switch (status) {
    case 'error':
      return { isError: true, content: [{ type: 'text', text: 'HTTP 502 Bad Gateway : le serveur amont ne répond pas.' }] }
    case 'denied':
      return { isError: true, content: [{ type: 'text', text: `Appel refusé par la politique : ${tool.policy.explanation}.` }] }
    case 'refused':
      return { isError: true, content: [{ type: 'text', text: "Appel refusé par l'utilisateur." }] }
    case 'timeout':
      return { isError: true, content: [{ type: 'text', text: 'Pas de réponse à la confirmation dans le délai : appel refusé.' }] }
    case 'blocked':
      return { isError: true, content: [{ type: 'text', text: 'Un secret connu figure dans les arguments : appel bloqué.' }] }
    case 'authRequired':
      return { isError: true, content: [{ type: 'text', text: 'Connexion requise : connectez-vous dans MCP Proxy.' }] }
    case 'rateLimited':
      return { isError: true, content: [{ type: 'text', text: 'Limite de débit atteinte : réessayez dans quelques secondes.' }] }
    case 'cancelled':
      return { isError: true, content: [{ type: 'text', text: 'Appel annulé par le client.' }] }
    case 'leakMasked':
      return { content: [{ type: 'text', text: 'Cookie: session=[SECRET MASQUÉ]; lang=fr' }] }
    default:
      return { content: [{ type: 'text', text: `${String(count)} résultats pour ${tool.name}.` }] }
  }
}

/** The `index`-th call of a working day, started at `at`, numbered `seq`. */
export function generatedCall(index: number, seq: number, at: number): JournalEntry {
  const draw = draws(index + 1)
  const all = toolsOf()
  const tool = all[Math.floor(draw() * all.length)] ?? all[0]
  if (!tool) throw new Error('snapshot.example.json has no tool')
  const client = CLIENTS[Math.floor(draw() * CLIENTS.length)] ?? 'Claude Desktop'
  const { status, slow } = statusOf(draw(), tool.policy)
  const decision = tool.policy.decision
  const durationMs = slow
    ? SLOW_MS + Math.floor(draw() * 9000)
    : ['denied', 'blocked', 'suspended'].includes(status)
      ? 1 + Math.floor(draw() * 4)
      : 40 + Math.floor(draw() * (status === 'timeout' ? 60_000 : 1400))
  const label = LABELS[status]
  const count = 1 + Math.floor(draw() * 40)
  return {
    seq,
    at,
    kind: 'call',
    status,
    statusLabel: label,
    slow,
    client,
    environment: tool.environment,
    mcp: tool.mcp,
    tool: tool.exposedName,
    durationMs,
    decision,
    summary: `${client} → ${tool.exposedName} : ${label}${slow ? ' (lent)' : ''}, ${String(durationMs)} ms`,
    problem: isProblem(status, slow),
    policy: isPolicy(status, decision),
    details: keepsContent(status, slow)
      ? {
          callId: 1000 + seq,
          client,
          session: tool.session,
          policy: tool.policy,
          request: { project: 'paiement/api', query: `${tool.name.replaceAll('_', ' ')} ${String(count)}`, limit: 50 },
          response: responseOf(status, tool, count),
          masked:
            status === 'leakMasked'
              ? { exact: 0, generic: 1, kinds: ['cookieHeader'] }
              : { exact: 0, generic: 0, kinds: [] },
        }
      : null,
  }
}

const EVENTS = [
  "MCP « Kibana Logs » coupé par l'utilisateur",
  "MCP « Kibana Logs » rétabli par l'utilisateur",
  "environnement « PPROD » exposé par l'utilisateur",
  "environnement « PPROD » retiré par l'utilisateur",
]

/** An event of the application (`kind: event`). */
export function eventEntry(seq: number, at: number, summary: string): JournalEntry {
  return {
    seq,
    at,
    kind: 'event',
    status: null,
    statusLabel: null,
    slow: false,
    client: null,
    environment: null,
    mcp: null,
    tool: null,
    durationMs: null,
    decision: null,
    summary,
    problem: false,
    policy: false,
    details: null,
  }
}

/** `count` entries numbered from 1, the last one shortly before `end`: mostly calls, a few events. */
export function workingDay(count: number, end: number, spacingMs = 45_000): JournalEntry[] {
  const entries: JournalEntry[] = []
  for (let index = 0; index < count; index += 1) {
    const seq = index + 1
    const at = end - (count - index) * spacingMs + Math.floor(draws(index + 7)() * spacingMs * 0.8)
    entries.push(
      index % 41 === 20
        ? eventEntry(seq, at, EVENTS[Math.floor(index / 41) % EVENTS.length] ?? '')
        : generatedCall(index, seq, at),
    )
  }
  return entries
}

/** How long the entries of the contract file last, from the first call to the last event. */
export const EXAMPLE_SPAN_MS = 125_000

/** The entries of the contract file after `before`, renumbered and dated just before `now`. */
export function withExamples(before: JournalEntry[], now: number): JournalEntry[] {
  const examples = exampleEntries()
  const last = examples[examples.length - 1]?.at ?? 0
  return [
    ...before,
    ...examples.map((e, index) => ({ ...e, seq: before.length + index + 1, at: now - 2000 - (last - e.at) })),
  ]
}
