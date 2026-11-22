// The snapshot the application sends to the main window (`get_snapshot`, `snapshot-changed`): the exact mirror of
// `mcp_proxy_gateway::view::AppSnapshot`, serialized in camelCase. Nothing in it is secret.
//
// Every object is strict: a key added, removed or retyped on the Rust side makes the snapshot unreadable here, and
// `snapshot.example.json` (written by the `snapshot_contract` test of the gateway) fails the tests of `ui/`.

import * as v from 'valibot'

const text = v.string()
const nullableText = v.nullable(v.string())
/** Milliseconds since the Unix epoch. */
const instant = v.nullable(v.pipe(v.number(), v.integer(), v.minValue(0)))
const count = v.pipe(v.number(), v.integer(), v.minValue(0))

export const AppStatusSchema = v.picklist(['neutral', 'attention', 'error', 'suspended'])
export const AlertLevelSchema = v.picklist(['info', 'attention', 'error'])
export const AlertKindSchema = v.picklist([
  'settingsReset',
  'toolThreshold',
  'catalogCache',
  'startupHardening',
  'settingsNotSaved',
])
export const SessionStateKindSchema = v.picklist(['neverConnected', 'active', 'expired', 'loginRequested'])
export const AuthMethodSchema = v.picklist(['jwtHeader', 'cookie', 'basic'])
export const CategorySchema = v.picklist(['read', 'write', 'destructive'])
export const InstanceStateSchema = v.picklist(['catalogue', 'connected', 'sessionRequired', 'unreachable', 'disabled'])
export const CatalogSourceSchema = v.picklist(['embedded', 'cache', 'live'])
export const DecisionKindSchema = v.picklist(['allow', 'confirmElicitation', 'confirmNotification', 'deny'])

export const AlertSchema = v.strictObject({
  level: AlertLevelSchema,
  kind: AlertKindSchema,
  message: text,
})

export const ProfileChoiceSchema = v.strictObject({
  id: text,
  label: text,
  description: text,
  strictest: v.boolean(),
  defaultEnvironments: v.array(text),
})

export const ProfileViewSchema = v.strictObject({
  chosen: nullableText,
  active: nullableText,
  firstLaunch: v.boolean(),
  choices: v.array(ProfileChoiceSchema),
})

export const EnvironmentViewSchema = v.strictObject({
  id: text,
  label: text,
  description: text,
  sensitive: v.boolean(),
  exposed: v.boolean(),
})

export const SessionStateViewSchema = v.strictObject({
  kind: SessionStateKindSchema,
  /** Computed in UTC by the core: the window formats `at` in local time instead. */
  label: text,
  at: instant,
  /** The LLM client waiting for the login; empty when unknown, `null` when nobody waits. */
  requestedBy: nullableText,
})

export const ServiceViewSchema = v.strictObject({
  environment: text,
  mcp: text,
})

export const SessionViewSchema = v.strictObject({
  id: text,
  label: text,
  /** `null` for a global session. */
  environment: nullableText,
  method: AuthMethodSchema,
  state: SessionStateViewSchema,
  services: v.array(ServiceViewSchema),
})

/** A decision of the policy engine and why, in French (the text of the journal). */
export const PolicyViewSchema = v.strictObject({
  decision: DecisionKindSchema,
  label: text,
  explanation: text,
})

/** Default policy of an instance: the decision for a declared tool of each category without a rule of its own. */
export const PolicySummaryViewSchema = v.strictObject({
  read: PolicyViewSchema,
  write: PolicyViewSchema,
  destructive: PolicyViewSchema,
})

export const ToolViewSchema = v.strictObject({
  exposedName: text,
  name: text,
  title: text,
  /** Given by the server, already filtered; empty without one. Shown as text, never as HTML. */
  description: text,
  category: CategorySchema,
  known: v.boolean(),
  exposed: v.boolean(),
  policy: PolicyViewSchema,
})

export const UpdateViewSchema = v.strictObject({
  added: v.array(text),
  modified: v.array(text),
  removed: v.array(text),
  at: instant,
})

export const InstanceViewSchema = v.strictObject({
  environment: text,
  exposed: v.boolean(),
  /** The session its calls use; `null` without one. */
  session: nullableText,
  policy: PolicySummaryViewSchema,
  state: InstanceStateSchema,
  stateLabel: text,
  source: v.nullable(CatalogSourceSchema),
  refreshedAt: instant,
  lastError: nullableText,
  tools: v.array(ToolViewSchema),
  update: v.nullable(UpdateViewSchema),
})

export const McpViewSchema = v.strictObject({
  id: text,
  alias: text,
  label: text,
  local: v.boolean(),
  disabled: v.boolean(),
  instances: v.array(InstanceViewSchema),
})

export const AppSnapshotSchema = v.strictObject({
  version: text,
  status: AppStatusSchema,
  tooltip: text,
  profile: ProfileViewSchema,
  environments: v.array(EnvironmentViewSchema),
  sessions: v.array(SessionViewSchema),
  mcps: v.array(McpViewSchema),
  exposedTools: count,
  exposedToolsThreshold: count,
  gatewaySuspended: v.boolean(),
  pendingConfirmations: count,
  alerts: v.array(AlertSchema),
  /** Update ready to apply (spec 020); always `null` before it. */
  update: nullableText,
})

export type AppStatus = v.InferOutput<typeof AppStatusSchema>
export type Alert = v.InferOutput<typeof AlertSchema>
export type ProfileChoice = v.InferOutput<typeof ProfileChoiceSchema>
export type ProfileView = v.InferOutput<typeof ProfileViewSchema>
export type EnvironmentView = v.InferOutput<typeof EnvironmentViewSchema>
export type SessionStateKind = v.InferOutput<typeof SessionStateKindSchema>
export type SessionStateView = v.InferOutput<typeof SessionStateViewSchema>
export type ServiceView = v.InferOutput<typeof ServiceViewSchema>
export type SessionView = v.InferOutput<typeof SessionViewSchema>
export type Category = v.InferOutput<typeof CategorySchema>
export type InstanceState = v.InferOutput<typeof InstanceStateSchema>
export type DecisionKind = v.InferOutput<typeof DecisionKindSchema>
export type PolicyView = v.InferOutput<typeof PolicyViewSchema>
export type PolicySummaryView = v.InferOutput<typeof PolicySummaryViewSchema>
export type ToolView = v.InferOutput<typeof ToolViewSchema>
export type UpdateView = v.InferOutput<typeof UpdateViewSchema>
export type InstanceView = v.InferOutput<typeof InstanceViewSchema>
export type McpView = v.InferOutput<typeof McpViewSchema>
export type AppSnapshot = v.InferOutput<typeof AppSnapshotSchema>

// The journal (spec 018): `get_journal` and each `journal-entry`, the exact mirror of
// `mcp_proxy_gateway::journal_view`. Already filtered by the core; the window shows it as text and filters nothing.

export const EntryKindSchema = v.picklist(['call', 'toolsChanged', 'panic', 'event'])
export const CallStatusSchema = v.picklist([
  'ok',
  'leakMasked',
  'error',
  'denied',
  'refused',
  'timeout',
  'blocked',
  'rateLimited',
  'authRequired',
  'suspended',
  'inputRequired',
  'cancelled',
])
export const MaskedKindSchema = v.picklist([
  'knownSecret',
  'jwt',
  'authorizationHeader',
  'cookieHeader',
  'sensitiveField',
  'privateKey',
  'urlCredentials',
  'other',
])

/** What the output filter masked: counts and kinds, never a value. */
export const MaskedViewSchema = v.strictObject({
  exact: count,
  generic: count,
  kinds: v.array(MaskedKindSchema),
})

/** The unfolded part of a row: only for a call whose content the core kept. */
export const DetailsViewSchema = v.strictObject({
  callId: count,
  client: text,
  session: nullableText,
  policy: v.nullable(PolicyViewSchema),
  /** Arguments and response, filtered JSON of any shape: shown as text. */
  request: v.unknown(),
  response: v.unknown(),
  masked: MaskedViewSchema,
})

export const EntryViewSchema = v.strictObject({
  seq: count,
  at: v.pipe(v.number(), v.integer(), v.minValue(0)),
  kind: EntryKindSchema,
  status: v.nullable(CallStatusSchema),
  statusLabel: nullableText,
  slow: v.boolean(),
  client: nullableText,
  /** Identifiers: the window shows the labels of the snapshot. */
  environment: nullableText,
  mcp: nullableText,
  tool: nullableText,
  durationMs: v.nullable(count),
  decision: v.nullable(DecisionKindSchema),
  summary: text,
  /** Filters `Problèmes` and `Politiques`, computed by the core. */
  problem: v.boolean(),
  policy: v.boolean(),
  details: v.nullable(DetailsViewSchema),
})

export const JournalViewSchema = v.strictObject({
  entries: v.array(EntryViewSchema),
})

export const ExportOutcomeSchema = v.variant('status', [
  v.strictObject({ status: v.literal('saved'), path: text }),
  v.strictObject({ status: v.literal('cancelled') }),
])

export type EntryKind = v.InferOutput<typeof EntryKindSchema>
export type CallStatus = v.InferOutput<typeof CallStatusSchema>
export type MaskedKind = v.InferOutput<typeof MaskedKindSchema>
export type MaskedView = v.InferOutput<typeof MaskedViewSchema>
export type DetailsView = v.InferOutput<typeof DetailsViewSchema>
export type JournalEntry = v.InferOutput<typeof EntryViewSchema>
export type JournalView = v.InferOutput<typeof JournalViewSchema>
export type ExportOutcome = v.InferOutput<typeof ExportOutcomeSchema>

/** The entries of `get_journal`, or `null` if `value` is not exactly a journal. */
export function parseJournal(value: unknown): JournalEntry[] | null {
  const result = v.safeParse(JournalViewSchema, value)
  return result.success ? result.output.entries : null
}

/** One entry of `journal-entry`, or `null`. */
export function parseEntry(value: unknown): JournalEntry | null {
  const result = v.safeParse(EntryViewSchema, value)
  return result.success ? result.output : null
}

/** The answer of `export_diagnostic`, or `null`. */
export function parseExportOutcome(value: unknown): ExportOutcome | null {
  const result = v.safeParse(ExportOutcomeSchema, value)
  return result.success ? result.output : null
}

/** The snapshot, or `null` if `value` is not exactly one. */
export function parseSnapshot(value: unknown): AppSnapshot | null {
  const result = v.safeParse(AppSnapshotSchema, value)
  return result.success ? result.output : null
}
