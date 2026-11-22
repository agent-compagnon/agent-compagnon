import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import type { AppSnapshot } from '@/api/types'
import { activeUntil } from '@/lib/time'
import { STORE, createStore } from '@/store/snapshot'
import { NOW, snapshotOf, spyBackend } from '@/test/fixtures'
import ConnectionsTab from './ConnectionsTab.vue'
import { servicesText } from './model'

function mountTab(snapshot: AppSnapshot) {
  const spy = spyBackend(snapshot)
  const wrapper = mount(ConnectionsTab, {
    props: { snapshot, now: NOW },
    global: { provide: { [STORE as symbol]: createStore(spy.backend) } },
  })
  return { wrapper, backend: spy.backend }
}

function card(wrapper: ReturnType<typeof mountTab>['wrapper'], session: string) {
  return wrapper.get(`[data-session="${session}"]`)
}

describe('Connections tab', () => {
  // Exigence : EX-016-01
  it('shows each state: dot, words and button', async () => {
    const snapshot = snapshotOf('connexion-demandee')
    const { wrapper, backend } = mountTab(snapshot)
    const expected = [
      ['sso-corp', 'active', activeUntil(NOW + 3 * 3_600_000 + 42 * 60_000, NOW), 'Déconnecter'],
      ['sso-tools', 'expired', 'expirée', 'Se connecter'],
      ['sso-prod', 'loginRequested', 'connexion demandée', 'Se connecter'],
      ['sso-pra', null, null, null],
    ] as const
    for (const [id, dot, state, button] of expected) {
      if (dot === null) {
        expect(wrapper.find(`[data-session="${id}"]`).exists(), `${id} is not exposed`).toBe(false)
        continue
      }
      const c = card(wrapper, id)
      expect(c.get('[data-dot]').attributes('data-dot')).toBe(dot)
      expect(c.get('[data-field="state"]').text()).toBe(state)
      expect(c.get('button').text()).toBe(button)
    }
    expect(card(wrapper, 'sso-prod').get('[data-field="state"]').classes()).toContain('text-attention-text')
    // Never connected: outline dot.
    const never = mountTab(snapshotOf('sessions')).wrapper
    expect(card(never, 'sso-int').get('[data-dot]').attributes('data-dot')).toBe('neverConnected')
    expect(card(never, 'sso-int').get('[data-field="state"]').text()).toBe('jamais connectée')

    await card(wrapper, 'sso-corp').get('button').trigger('click')
    expect(backend.disconnectSession).toHaveBeenCalledWith('sso-corp')
    await card(wrapper, 'sso-tools').get('button').trigger('click')
    expect(backend.connectSession).toHaveBeenCalledWith('sso-tools')
  })

  // Exigence : EX-016-01
  it('shows the global sessions, then one card per exposed environment', () => {
    const { wrapper } = mountTab(snapshotOf('sessions'))
    const global = wrapper.findAll('#sessions-global ~ ul [data-session]').map((c) => c.attributes('data-session'))
    expect(global).toEqual(['sso-corp', 'sso-tools'])
    const perEnv = wrapper.findAll('#sessions-env ~ ul [data-session]').map((c) => c.attributes('data-session'))
    expect(perEnv).toEqual(['sso-dev', 'sso-int', 'sso-rec'])

    const none = snapshotOf('sessions')
    none.environments = none.environments.map((e) => ({ ...e, exposed: false }))
    const empty = mountTab(none).wrapper
    expect(empty.find('#sessions-env ~ ul').exists()).toBe(false)
    expect(empty.get('[data-empty="environments"]').text()).toBe("Aucun environnement n'est exposé aux clients LLM.")
    expect(card(empty, 'sso-corp').get('[data-field="services"]').text()).toContain('Aucun service exposé')
  })

  // Exigence : EX-016-01
  it('names the services a session covers, in exposed environments only', () => {
    const snapshot = snapshotOf('sessions')
    const corp = snapshot.sessions.find((s) => s.id === 'sso-corp')
    const dev = snapshot.sessions.find((s) => s.id === 'sso-dev')
    if (!corp || !dev) throw new Error('example sessions')
    expect(servicesText(corp, snapshot)).toBe('GitLab Ops en DEV, REC · Base de données (lecture) en INT, REC')
    expect(servicesText(dev, snapshot)).toBe('Kibana Logs, Kubernetes Admin')
  })

  // Exigence : EX-016-01
  it('shows the orange banner only while an LLM client waits for a login', async () => {
    const waiting = mountTab(snapshotOf('connexion-demandee'))
    const banner = waiting.wrapper.get('[data-banner="login"]')
    expect(banner.text()).toContain('Claude Desktop attend une connexion à SSO PROD.')
    expect(banner.attributes('data-tone')).toBe('attention')
    await banner.get('button').trigger('click')
    expect(waiting.backend.connectSession).toHaveBeenCalledWith('sso-prod')

    expect(mountTab(snapshotOf('sessions')).wrapper.find('[data-banner="login"]').exists()).toBe(false)
    // Asked from the window only: nobody waits, no banner, the card alone is orange.
    const own = snapshotOf('sessions')
    own.sessions = own.sessions.map((s) =>
      s.id === 'sso-int' ? { ...s, state: { kind: 'loginRequested', label: '', at: NOW, requestedBy: null } } : s,
    )
    const ownTab = mountTab(own).wrapper
    expect(ownTab.find('[data-banner="login"]').exists()).toBe(false)
    expect(card(ownTab, 'sso-int').get('[data-dot]').attributes('data-dot')).toBe('loginRequested')
    // A client without a name.
    const unknown = snapshotOf('connexion-demandee')
    unknown.sessions = unknown.sessions.map((s) =>
      s.id === 'sso-prod' ? { ...s, state: { ...s.state, requestedBy: '' } } : s,
    )
    expect(mountTab(unknown).wrapper.get('[data-banner="login"]').text()).toContain(
      'Un client LLM attend une connexion à SSO PROD.',
    )
  })

  // Exigence : EX-016-01
  it('writes the local time of the end of a session, from its instant', () => {
    const at = new Date(NOW)
    at.setHours(17, 42, 0, 0)
    expect(activeUntil(at.getTime(), NOW)).toBe("active jusqu'à 17:42")
    const tomorrow = new Date(at)
    tomorrow.setDate(at.getDate() + 1)
    tomorrow.setHours(9, 5)
    expect(activeUntil(tomorrow.getTime(), NOW)).toBe("active jusqu'à demain 09:05")
    const later = new Date(at)
    later.setDate(at.getDate() + 3)
    expect(activeUntil(later.getTime(), NOW)).toMatch(/^active jusqu'à 25\/11 17:42$/)
  })
})
