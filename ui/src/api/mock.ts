// Simulated backend, to run the window in a plain browser (development, tests, screenshots). Built only in
// `--mode mock` (`npm run dev`, `npm run build:mock`): the application's build never contains it, and
// `scripts/check-dist.mjs` fails the build if it does.
//
// Its data is the snapshot of the example configuration written by the gateway (`snapshot.example.json`), reshaped
// by scenario. Choose one with `?scenario=…`.

import type { Backend } from './backend'
import example from './snapshot.example.json'
import { type AppSnapshot, type SessionStateView, parseSnapshot } from './types'

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
] as const

export type Scenario = (typeof SCENARIOS)[number]

const HOUR = 3_600_000
const LOGIN_DELAY_MS = 1500

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
      return snapshot
    }
  }
}

/** Status of the icon, as `AppStatus::of` computes it. */
function withStatus(snapshot: AppSnapshot): AppSnapshot {
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

export function scenarioOf(search: string): Scenario {
  const name = new URLSearchParams(search).get('scenario')
  return SCENARIOS.find((s) => s === name) ?? 'sessions'
}

/** A backend that answers like the application, from `scenario`, with a login that succeeds after a moment. */
export function createMockBackend(name: Scenario, clock: () => number = Date.now): Backend {
  let current = withStatus(scenario(name, clock()))
  const listeners = new Set<(snapshot: AppSnapshot) => void>()

  function publish(change: (snapshot: AppSnapshot) => void) {
    const next = structuredClone(current)
    change(next)
    current = withStatus(next)
    for (const listener of listeners) listener(structuredClone(current))
  }

  function known(session: string) {
    if (!current.sessions.some((s) => s.id === session)) throw new Error('session inconnue')
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
      return Promise.resolve()
    },
    showConfirmations() {
      // The detail windows belong to the application; nothing is answered here either.
      console.info(MOCK_MARKER, 'show_confirmations')
      return Promise.resolve()
    },
  }
}
