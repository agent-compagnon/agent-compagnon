// What the Connections tab shows, computed from the snapshot. Pure: tested without any component.

import { fr } from '@/i18n/fr'
import { activeUntil } from '@/lib/time'
import type { AppSnapshot, SessionView } from '@/api/types'

/** The state in words; the label of the core is not used (computed in UTC). */
export function stateText(session: SessionView, now: number): string {
  const state = session.state
  switch (state.kind) {
    case 'neverConnected':
      return fr.connections.state.neverConnected
    case 'active':
      return state.at === null ? fr.connections.state.active('—') : activeUntil(state.at, now)
    case 'expired':
      return fr.connections.state.expired
    case 'loginRequested':
      return fr.connections.state.loginRequested
  }
}

/** « Déconnecter » for an active session, « Se connecter » otherwise. */
export function canDisconnect(session: SessionView): boolean {
  return session.state.kind === 'active'
}

/** The global sessions, then one per exposed environment, in the order of the configuration. */
export function sessionGroups(snapshot: AppSnapshot): { global: SessionView[]; perEnvironment: SessionView[] } {
  const exposed = new Set(snapshot.environments.filter((e) => e.exposed).map((e) => e.id))
  return {
    global: snapshot.sessions.filter((s) => s.environment === null),
    perEnvironment: snapshot.sessions.filter((s) => s.environment !== null && exposed.has(s.environment)),
  }
}

/**
 * The services a session covers, in exposed environments only (what the LLM clients reach), in the order of the
 * configuration: MCP labels, with their environments for a global session. « Kibana Logs, GitLab Ops » ;
 * « GitLab Ops en DEV, REC · Jira en REC ».
 */
export function servicesText(session: SessionView, snapshot: AppSnapshot): string {
  const exposed = snapshot.environments.filter((e) => e.exposed)
  const covered = (mcp: string) =>
    exposed.filter((e) => session.services.some((s) => s.mcp === mcp && s.environment === e.id)).map((e) => e.label)
  const mcps = snapshot.mcps.flatMap((m) => {
    const envs = covered(m.id)
    return envs.length === 0 ? [] : [{ label: m.label, envs }]
  })
  if (mcps.length === 0) return fr.connections.noService
  if (session.environment !== null) return mcps.map((m) => m.label).join(', ')
  return mcps.map((m) => fr.connections.serviceIn(m.label, m.envs)).join(' · ')
}

export interface LoginBanner {
  session: string
  text: string
}

/** One orange banner per login an LLM client waits for. */
export function loginBanners(snapshot: AppSnapshot): LoginBanner[] {
  return snapshot.sessions.flatMap((s) => {
    const client = s.state.requestedBy
    if (s.state.kind !== 'loginRequested' || client === null) return []
    return [{ session: s.id, text: fr.banners.loginWaiting(client === '' ? fr.banners.unknownClient : client, s.label) }]
  })
}
