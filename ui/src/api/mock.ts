// Simulated backend, to run the window in a plain browser (development, tests, screenshots). Built only in
// `--mode mock` (`npm run dev`, `npm run build:mock`): the application's build never contains it, and
// `scripts/check-dist.mjs` fails the build if it does.
//
// Its data is the snapshot of the example configuration written by the gateway (`snapshot.example.json`), reshaped
// by scenario, and a journal made from the example entries (`journal.example.json`) and generated calls. Choose a
// scenario with `?scenario=…`.

import type { Backend, JournalEvent } from './backend'
import { CLIENT_SCENARIOS, type ClientScenario, createMockClients } from './mock-clients'
import doctorExample from './doctor.example.json'
import { EXAMPLE_SPAN_MS, eventEntry, generatedCall, withExamples, workingDay } from './mock-journal'
import example from './snapshot.example.json'
import {
  type AppSnapshot,
  type DoctorReport,
  type JournalEntry,
  type McpView,
  type SessionStateView,
  type ToolView,
  parseDoctorReport,
  parseSnapshot,
} from './types'

/** Marker searched for in the application's build by `scripts/check-dist.mjs`. */
export const MOCK_MARKER = 'mcp-proxy-simulated-backend'

export const SCENARIOS = [
  'premier-lancement',
  'sessions',
  'connexion-demandee',
  'confirmation',
  'suspendue',
  'reglages-reinitialises',
  'profil-webview',
  'mcp-mis-a-jour',
  'seuil',
  'volumineux',
  'rien-expose',
  'journal-direct',
  'journal-volumineux',
  'journal-vide',
  'export-echec',
  'export-annule',
  'mise-a-jour-prete',
  'mise-a-jour-critique',
  ...CLIENT_SCENARIOS,
  'diagnostic-ok',
  'diagnostic-avertissements',
  'diagnostic-echecs',
  'diagnostic-en-cours',
] as const

export type Scenario = (typeof SCENARIOS)[number]

const HOUR = 3_600_000
const LOGIN_DELAY_MS = 1500
/** A new call in `journal-direct`. */
export const LIVE_PERIOD_MS = 1200
const EXPORT_DELAY_MS = 600
/** The checks run together, bounded: a short wait shows « Contrôle en cours… ». */
export const DOCTOR_DELAY_MS = 900
/** Entries of `journal-volumineux`: as many as the window keeps. */
export const LARGE_JOURNAL = 5000

function base(): AppSnapshot {
  const snapshot = parseSnapshot(structuredClone(example))
  if (snapshot === null) throw new Error('snapshot.example.json does not match types.ts')
  return snapshot
}

function state(kind: SessionStateView['kind'], at: number | null = null, requestedBy: string | null = null) {
  const labels = {
    neverConnected: 'jamais connectée',
    active: 'active',
    expired: 'expirée',
    loginRequested: 'connexion demandée',
  }
  return { kind, label: labels[kind], at, requestedBy }
}

/** The profile chosen, its default environments exposed; every session never connected, nothing waits. */
function withProfile(snapshot: AppSnapshot, profile: string | null): AppSnapshot {
  const choice = snapshot.profile.choices.find((p) => p.id === profile)
  const strictest = snapshot.profile.choices.find((p) => p.strictest)
  const exposed = new Set(choice?.defaultEnvironments ?? [])
  snapshot.profile = {
    ...snapshot.profile,
    chosen: choice?.id ?? null,
    active: choice?.id ?? strictest?.id ?? null,
    firstLaunch: choice === undefined,
  }
  snapshot.environments = snapshot.environments.map((e) => ({ ...e, exposed: exposed.has(e.id) }))
  snapshot.mcps = snapshot.mcps.map((m) => ({
    ...m,
    instances: m.instances.map((i) => ({
      ...i,
      exposed: exposed.has(i.environment),
      update: null,
      lastError: null,
      tools: i.tools.map((t) => ({ ...t, exposed: exposed.has(i.environment) })),
    })),
  }))
  snapshot.sessions = snapshot.sessions.map((s) => ({ ...s, state: state('neverConnected') }))
  snapshot.alerts = []
  snapshot.pendingConfirmations = 0
  snapshot.gatewaySuspended = false
  snapshot.update = null
  return snapshot
}

function setState(snapshot: AppSnapshot, session: string, value: SessionStateView) {
  snapshot.sessions = snapshot.sessions.map((s) => (s.id === session ? { ...s, state: value } : s))
}

/** Sessions in every state, as after a working morning. */
function working(now: number, profile = 'developpeur'): AppSnapshot {
  const snapshot = withProfile(base(), profile)
  setState(snapshot, 'sso-corp', state('active', now + 3 * HOUR + 42 * 60_000))
  setState(snapshot, 'sso-tools', state('expired'))
  setState(snapshot, 'sso-dev', state('active', now + 5 * HOUR + 10 * 60_000))
  setState(snapshot, 'sso-rec', state('expired'))
  setState(snapshot, 'sso-pprod', state('active', now + 20 * HOUR))
  return snapshot
}

const ERROR_ALERTS = {
  settingsReset: {
    level: 'error' as const,
    kind: 'settingsReset' as const,
    message:
      "Vos réglages n'ont pas pu être vérifiés (réglages refusés : sceau invalide) : les réglages les plus stricts les remplacent, aucun environnement n'est exposé. Choisissez de nouveau votre profil.",
  },
  startupHardening: {
    level: 'error' as const,
    kind: 'startupHardening' as const,
    message:
      "Le dossier des fenêtres de connexion n'a pas pu être vidé (accès refusé) : aucune connexion par fenêtre n'est possible jusqu'au prochain démarrage de MCP Proxy.",
  },
}

export function scenario(name: Scenario, now: number): AppSnapshot {
  switch (name) {
    case 'premier-lancement':
      return withProfile(base(), null)
    case 'sessions':
      return working(now)
    case 'connexion-demandee': {
      const snapshot = working(now, 'support')
      setState(snapshot, 'sso-prod', state('loginRequested', now - 4000, 'Claude Desktop'))
      return snapshot
    }
    case 'confirmation': {
      const snapshot = working(now, 'support')
      snapshot.pendingConfirmations = 1
      return snapshot
    }
    case 'suspendue': {
      const snapshot = working(now)
      snapshot.gatewaySuspended = true
      return snapshot
    }
    case 'reglages-reinitialises': {
      const snapshot = withProfile(base(), null)
      snapshot.alerts = [ERROR_ALERTS.settingsReset]
      return snapshot
    }
    case 'profil-webview': {
      const snapshot = working(now)
      snapshot.alerts = [ERROR_ALERTS.startupHardening]
      return snapshot
    }
    case 'mcp-mis-a-jour': {
      const snapshot = working(now)
      const fresh = base()
      snapshot.mcps = snapshot.mcps.map((m) => ({
        ...m,
        instances: m.instances.map((i) => ({
          ...i,
          update:
            fresh.mcps.find((f) => f.id === m.id)?.instances.find((f) => f.environment === i.environment)?.update ??
            null,
        })),
      }))
      addNewTools(snapshot, 'kibana-logs', 'rec', now)
      return snapshot
    }
    case 'seuil': {
      const snapshot = working(now)
      snapshot.environments = snapshot.environments.map((e) => ({ ...e, exposed: true }))
      return snapshot
    }
    case 'volumineux': {
      const snapshot = working(now, 'support')
      snapshot.environments = snapshot.environments.map((e) => ({ ...e, exposed: e.id !== 'pra' }))
      snapshot.mcps.push(synthetic(snapshot, 'grafana', 'grafana', 'Grafana Dashboards', GRAFANA_TOOLS))
      snapshot.mcps.push(synthetic(snapshot, 'vault-read', 'vault', 'Vault (lecture)', VAULT_TOOLS))
      return snapshot
    }
    case 'rien-expose': {
      const snapshot = working(now)
      snapshot.environments = snapshot.environments.map((e) => ({ ...e, exposed: false }))
      return snapshot
    }
    case 'mise-a-jour-prete':
    case 'mise-a-jour-critique': {
      const snapshot = working(now)
      snapshot.update = { version: '0.2.0', critical: name === 'mise-a-jour-critique' }
      return snapshot
    }
    case 'journal-direct':
    case 'journal-volumineux':
    case 'journal-vide':
    case 'export-echec':
    case 'export-annule':
    case 'clients-etats':
    case 'clients-deux-emplacements':
    case 'clients-illisible':
    case 'clients-non-installe':
    case 'clients-modifie':
    case 'diagnostic-ok':
    case 'diagnostic-avertissements':
    case 'diagnostic-echecs':
    case 'diagnostic-en-cours':
      return working(now)
  }
}

/** The report of a scenario, as the gateway writes it for the contract. Every other scenario: all is well. */
export function doctorOf(name: Scenario): DoctorReport {
  const key = name === 'diagnostic-avertissements' ? 'warnings' : name === 'diagnostic-echecs' ? 'failures' : 'ok'
  const report = parseDoctorReport(structuredClone(doctorExample[key]))
  if (report === null) throw new Error('doctor.example.json does not match types.ts')
  return report
}

/** The journal of a scenario as `get_journal` first gives it, oldest first. */
export function journalOf(name: Scenario, now: number): JournalEntry[] {
  switch (name) {
    case 'journal-vide':
      return []
    case 'journal-volumineux':
      return withExamples(workingDay(LARGE_JOURNAL - 10, now - EXAMPLE_SPAN_MS, 9000), now)
    default:
      return withExamples(workingDay(50, now - EXAMPLE_SPAN_MS), now)
  }
}

/** `mcp-proxy-diagnostic-AAAAMMJJ-HHMM.zip`, in UTC like the core. */
function archiveName(at: number): string {
  const iso = new Date(at).toISOString()
  return `mcp-proxy-diagnostic-${iso.slice(0, 10).replaceAll('-', '')}-${iso.slice(11, 16).replace(':', '')}.zip`
}

/** Two tools listed by the server since the last visit, unknown to the configuration. */
function addNewTools(snapshot: AppSnapshot, mcp: string, environment: string, now: number) {
  const instance = snapshot.mcps.find((m) => m.id === mcp)?.instances.find((i) => i.environment === environment)
  const label = snapshot.environments.find((e) => e.id === environment)?.label ?? environment
  if (!instance) return
  const fresh: [string, string][] = [
    ['export_logs', 'Exports the matching log entries as a CSV file.'],
    ['tail_logs', 'Streams the newest log entries of an index for a few seconds.'],
  ]
  for (const [name, description] of fresh) {
    instance.tools.push({
      exposedName: `${environment}_kibana_${name}`,
      name,
      title: `${label} · Kibana Logs · ${name}`,
      description,
      category: 'destructive',
      known: false,
      exposed: instance.exposed,
      policy: {
        decision: 'confirmNotification',
        label: 'confirmation (notification)',
        explanation: 'confirmation (notification) — outil inconnu de la config, profil « Développeur »',
      },
    })
  }
  instance.state = 'connected'
  instance.source = 'live'
  instance.refreshedAt = now - 120_000
  instance.update = { added: fresh.map(([n]) => n), modified: ['search_logs'], removed: [], at: now - 120_000 }
}

const GRAFANA_TOOLS = [
  'list_dashboards',
  'get_dashboard',
  'search_panels',
  'query_datasource',
  'list_datasources',
  'get_alert_rules',
  'list_alerts',
  'silence_alert',
  'create_annotation',
  'list_annotations',
  'get_panel_image',
  'list_folders',
  'get_folder',
  'list_teams',
  'get_team',
  'list_users',
  'get_user',
  'update_dashboard',
  'delete_dashboard',
  'export_dashboard',
]

const VAULT_TOOLS = [
  'list_mounts',
  'list_secrets_paths',
  'read_metadata',
  'list_policies',
  'read_policy',
  'list_roles',
  'read_role',
  'audit_devices',
  'health',
  'leader',
  'seal_status',
  'list_leases',
]

/** An MCP that the example configuration does not have, in every environment, to see the table at scale. */
function synthetic(snapshot: AppSnapshot, id: string, alias: string, label: string, names: string[]): McpView {
  const model = snapshot.mcps.find((m) => m.id === 'kibana-logs')
  return {
    id,
    alias,
    label,
    local: false,
    disabled: false,
    instances: snapshot.environments.map((environment) => {
      const like = model?.instances.find((i) => i.environment === environment.id)
      const policy = like?.policy ?? snapshot.mcps[0]?.instances[0]?.policy
      if (!policy) throw new Error('snapshot.example.json has no instance')
      const tools: ToolView[] = names.map((name) => {
        const category = /^(delete|silence|update|create)/.test(name)
          ? 'destructive'
          : /^export/.test(name)
            ? 'write'
            : 'read'
        return {
          exposedName: `${environment.id}_${alias}_${name}`,
          name,
          title: `${environment.label} · ${label} · ${name}`,
          description: `${name.replaceAll('_', ' ')} (${label}).`,
          category,
          known: true,
          exposed: false,
          policy: policy[category],
        }
      })
      return {
        environment: environment.id,
        exposed: false,
        session: like?.session ?? null,
        policy,
        state: 'catalogue' as const,
        stateLabel: 'catalogue',
        source: 'embedded' as const,
        refreshedAt: null,
        lastError: null,
        tools,
        update: null,
      }
    }),
  }
}

/**
 * Exposure, tool counter and threshold alert, as the gateway computes them: a tool is exposed when a profile is
 * chosen, its environment is exposed and its MCP is not cut. A cut MCP shows « coupé »; put back, it shows
 * « catalogue » (the gateway keeps the state of its last reload, the simulation does not).
 */
function withExposure(snapshot: AppSnapshot): AppSnapshot {
  const exposedEnvironments = new Set(
    snapshot.profile.chosen === null ? [] : snapshot.environments.filter((e) => e.exposed).map((e) => e.id),
  )
  let count = 0
  for (const mcp of snapshot.mcps) {
    for (const instance of mcp.instances) {
      instance.exposed = exposedEnvironments.has(instance.environment) && !mcp.disabled
      for (const tool of instance.tools) tool.exposed = instance.exposed
      if (instance.exposed) count += instance.tools.length
      if (mcp.disabled) instance.state = 'disabled'
      else if (instance.state === 'disabled') instance.state = 'catalogue'
    }
  }
  snapshot.exposedTools = count
  snapshot.alerts = snapshot.alerts.filter((a) => a.kind !== 'toolThreshold')
  if (count > snapshot.exposedToolsThreshold) {
    snapshot.alerts.push({
      level: 'attention',
      kind: 'toolThreshold',
      message: `${String(count)} outils exposés, au-delà du seuil de ${String(snapshot.exposedToolsThreshold)} : certains clients LLM en ignorent une partie.`,
    })
  }
  return snapshot
}

/** Status of the icon, as `AppStatus::of` computes it, once the exposure is computed. */
function withStatus(snapshot: AppSnapshot): AppSnapshot {
  withExposure(snapshot)
  const waiting = snapshot.sessions.filter((s) => s.state.kind === 'loginRequested').length
  const errors = snapshot.alerts.filter((a) => a.level === 'error').length
  snapshot.status = snapshot.gatewaySuspended
    ? 'suspended'
    : waiting > 0 || snapshot.pendingConfirmations > 0
      ? 'attention'
      : errors > 0
        ? 'error'
        : 'neutral'
  return snapshot
}

/** A command of the simulation: a thrown error is its refusal. */
function attempt(action: () => void): Promise<void> {
  try {
    action()
    return Promise.resolve()
  } catch (error) {
    return Promise.reject(error instanceof Error ? error : new Error(String(error)))
  }
}

/** The snapshot of a scenario as the simulated backend first sends it: exposure, counter and status computed. */
export function initialSnapshot(name: Scenario, now: number): AppSnapshot {
  return withStatus(scenario(name, now))
}

export function scenarioOf(search: string): Scenario {
  const name = new URLSearchParams(search).get('scenario')
  return SCENARIOS.find((s) => s === name) ?? 'sessions'
}

/** A backend that answers like the application, from `scenario`, with a login that succeeds after a moment. */
export function createMockBackend(name: Scenario, clock: () => number = Date.now): Backend {
  let current = initialSnapshot(name, clock())
  const listeners = new Set<(snapshot: AppSnapshot) => void>()
  const journal = journalOf(name, clock())
  const journalListeners = new Set<(event: JournalEvent) => void>()

  function record(entry: JournalEntry) {
    journal.push(entry)
    if (journal.length > LARGE_JOURNAL) journal.splice(0, journal.length - LARGE_JOURNAL)
    for (const listener of journalListeners) listener({ type: 'entry', entry: structuredClone(entry) })
  }
  const nextSeq = () => (journal[journal.length - 1]?.seq ?? 0) + 1
  function recordEvent(summary: string) {
    record(eventEntry(nextSeq(), clock(), summary))
  }
  if (name === 'journal-direct') {
    setInterval(() => {
      const seq = nextSeq()
      record(generatedCall(seq + 7919, seq, clock()))
    }, LIVE_PERIOD_MS)
  }
  const isClients = (n: Scenario): n is ClientScenario => (CLIENT_SCENARIOS as readonly string[]).includes(n)
  const clients = createMockClients(isClients(name) ? name : 'clients-etats', clock, recordEvent)
  const label = (items: readonly { id: string; label: string }[], id: string) =>
    items.find((i) => i.id === id)?.label ?? id

  function publish(change: (snapshot: AppSnapshot) => void) {
    const next = structuredClone(current)
    change(next)
    current = withStatus(next)
    for (const listener of listeners) listener(structuredClone(current))
  }

  function known(session: string) {
    if (!current.sessions.some((s) => s.id === session)) throw new Error('session inconnue')
  }

  function instance(environment: string, mcp: string) {
    if (!current.environments.some((e) => e.id === environment)) throw new Error('environnement inconnu')
    const found = current.mcps.find((m) => m.id === mcp)
    if (!found) throw new Error('MCP inconnu')
    const at = found.instances.find((i) => i.environment === environment)
    if (!at) throw new Error("ce MCP n'existe pas dans cet environnement")
    return { mcp: found, instance: at }
  }

  return {
    getSnapshot() {
      return Promise.resolve(structuredClone(current))
    },
    onSnapshot(listener) {
      listeners.add(listener)
      return Promise.resolve(() => listeners.delete(listener))
    },
    connectSession(session) {
      known(session)
      const kind = current.sessions.find((s) => s.id === session)?.state.kind
      if (kind === 'active' || kind === 'loginRequested') return Promise.resolve()
      publish((s) => {
        setState(s, session, state('loginRequested', clock()))
      })
      setTimeout(() => {
        publish((s) => {
          setState(s, session, state('active', clock() + 8 * HOUR))
        })
      }, LOGIN_DELAY_MS)
      return Promise.resolve()
    },
    disconnectSession(session) {
      known(session)
      publish((s) => {
        const kind = s.sessions.find((x) => x.id === session)?.state.kind
        if (kind !== 'neverConnected') setState(s, session, state('expired'))
      })
      return Promise.resolve()
    },
    setProfile(profile) {
      if (!current.profile.choices.some((p) => p.id === profile)) return Promise.reject(new Error('profil inconnu'))
      publish((s) => {
        const sessions = s.sessions
        const alerts = s.alerts.filter((a) => a.kind !== 'settingsReset')
        const pending = s.pendingConfirmations
        const suspended = s.gatewaySuspended
        withProfile(s, profile)
        recordEvent(`profil « ${label(s.profile.choices, profile)} » choisi par l'utilisateur`)
        s.sessions = sessions
        s.alerts = alerts
        s.pendingConfirmations = pending
        s.gatewaySuspended = suspended
      })
      return Promise.resolve()
    },
    resumeGateway() {
      publish((s) => {
        s.gatewaySuspended = false
      })
      recordEvent("passerelle reprise par l'utilisateur")
      return Promise.resolve()
    },
    showConfirmations() {
      // The detail windows belong to the application; nothing is answered here either.
      console.info(MOCK_MARKER, 'show_confirmations')
      return Promise.resolve()
    },
    setEnvironmentExposed(environment, exposed) {
      if (!current.environments.some((e) => e.id === environment))
        return Promise.reject(new Error('environnement inconnu'))
      if (exposed && current.profile.chosen === null) return Promise.reject(new Error("choisissez d'abord un profil"))
      publish((s) => {
        s.environments = s.environments.map((e) => (e.id === environment ? { ...e, exposed } : e))
      })
      const env = label(current.environments, environment)
      recordEvent(`environnement « ${env} » ${exposed ? 'exposé' : 'retiré'} par l'utilisateur`)
      return Promise.resolve()
    },
    setMcpExposed(mcp, exposed) {
      if (!current.mcps.some((m) => m.id === mcp)) return Promise.reject(new Error('MCP inconnu'))
      publish((s) => {
        s.mcps = s.mcps.map((m) => (m.id === mcp ? { ...m, disabled: !exposed } : m))
      })
      recordEvent(`MCP « ${label(current.mcps, mcp)} » ${exposed ? 'rétabli' : 'coupé'} par l'utilisateur`)
      return Promise.resolve()
    },
    acknowledgeMcpUpdate(environment, mcp) {
      return attempt(() => {
        instance(environment, mcp)
        publish((s) => {
          const target = s.mcps.find((m) => m.id === mcp)?.instances.find((i) => i.environment === environment)
          if (target) target.update = null
        })
      })
    },
    reloadMcp(environment, mcp) {
      return attempt(() => {
        const found = instance(environment, mcp)
        if (found.mcp.disabled) throw new Error("ce MCP est coupé, sa liste n'est pas rechargée")
        if (!found.instance.exposed)
          throw new Error("ce MCP n'est pas exposé dans cet environnement, sa liste n'est pas rechargée")
        publish((s) => {
          const target = s.mcps.find((m) => m.id === mcp)?.instances.find((i) => i.environment === environment)
          if (target) {
            target.state = 'connected'
            target.stateLabel = 'connecté'
            target.lastError = null
            target.source = 'live'
            target.refreshedAt = clock()
          }
        })
      })
    },
    getJournal() {
      return Promise.resolve(structuredClone(journal))
    },
    onJournal(listener) {
      journalListeners.add(listener)
      return Promise.resolve(() => journalListeners.delete(listener))
    },
    restartForUpdate() {
      const ready = current.update
      if (ready === null) return Promise.reject(new Error('aucune mise à jour prête à appliquer'))
      // The application would start its new version and stop: the simulation only records it.
      recordEvent(`redémarrage demandé pour passer à MCP Proxy ${ready.version}`)
      console.info(MOCK_MARKER, 'restart_for_update')
      return Promise.resolve()
    },
    runDoctor() {
      // The checks read the machine in the application; the simulation gives the reports of the contract.
      if (name === 'diagnostic-en-cours') return new Promise<DoctorReport>(() => undefined)
      return new Promise((resolve) => {
        setTimeout(() => {
          resolve(doctorOf(name))
        }, DOCTOR_DELAY_MS)
      })
    },
    getLlmClients: clients.getLlmClients,
    previewLlmClient: clients.previewLlmClient,
    applyLlmClient: clients.applyLlmClient,
    exportDiagnostic() {
      // The save dialog belongs to the application: the simulation answers as if a path had been chosen.
      return new Promise((resolve, reject) => {
        setTimeout(() => {
          if (name === 'export-annule') {
            resolve({ status: 'cancelled' })
          } else if (name === 'export-echec') {
            const reason = 'export de diagnostic impossible : accès refusé'
            recordEvent(reason)
            reject(new Error(reason))
          } else {
            recordEvent("diagnostic exporté par l'utilisateur")
            resolve({ status: 'saved', path: `C:\\Users\\exemple\\Downloads\\${archiveName(clock())}` })
          }
        }, EXPORT_DELAY_MS)
      })
    },
  }
}
