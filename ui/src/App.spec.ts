import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import { DropdownMenuRadioGroup } from 'reka-ui'
import type { AppSnapshot } from '@/api/types'
import { createStore } from '@/store/snapshot'
import { snapshotOf, spyBackend } from '@/test/fixtures'
import App from './App.vue'

async function mountApp(snapshot: AppSnapshot) {
  const spy = spyBackend(snapshot)
  const wrapper = mount(App, { props: { store: createStore(spy.backend) }, attachTo: document.body })
  await flushPromises()
  return { wrapper, ...spy }
}

/** The accessible name of a control: its label, the elements it is labelled by, or its text. */
function accessibleName(element: Element): string {
  const label = element.getAttribute('aria-label')
  if (label) return label
  const by = element.getAttribute('aria-labelledby')
  if (by) return by.split(' ').map((id) => document.getElementById(id)?.textContent.trim() ?? '').join(' ')
  return element.textContent.trim()
}

function inBody(selector: string): HTMLElement | null {
  return document.body.querySelector<HTMLElement>(selector)
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('main window', () => {
  // Exigence : EX-016-01
  it('has a header with the profile and the version, and three tabs, Connexions first', async () => {
    const { wrapper } = await mountApp(snapshotOf('sessions'))
    expect(wrapper.get('header h1').text()).toBe('MCP Proxy')
    expect(wrapper.get('header').text()).toContain('version 0.1.0')
    expect(wrapper.get('[data-action="profile-menu"]').attributes('aria-label')).toBe('Profil : Développeur')
    const tabs = wrapper.findAll('[role="tab"]')
    expect(tabs.map((t) => t.text())).toEqual(['Connexions', 'MCP', 'Journal'])
    expect(tabs[0]?.attributes('aria-selected')).toBe('true')
    expect(wrapper.find('[data-action="update"]').exists()).toBe(false)
    expect(inBody('[data-dialog="first-launch"]')).toBeNull()
    wrapper.unmount()
  })

  // Exigence : EX-016-01
  it('asks for the profile at first launch, and nothing closes the dialog but a choice', async () => {
    const { wrapper, backend } = await mountApp(snapshotOf('premier-lancement'))
    const dialog = inBody('[data-dialog="first-launch"]')
    expect(dialog).not.toBeNull()
    expect(dialog?.getAttribute('role')).toBe('alertdialog')
    const labels = [...(dialog?.querySelectorAll('[role="radio"]') ?? [])].map(accessibleName)
    expect(labels).toEqual(['Développeur', 'Support', 'Lecture seule'])
    expect(dialog?.textContent).toContain('Environnements exposés : REC, PPROD et PROD. Dont sensibles : PPROD et PROD.')
    expect(dialog?.textContent).toContain('Pour consulter sans risque')
    const button = inBody('[data-action="choose-profile"]') as HTMLButtonElement
    expect(button.disabled).toBe(true)

    dialog?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await flushPromises()
    expect(inBody('[data-dialog="first-launch"]')).not.toBeNull()

    inBody('#profile-support')?.click()
    await flushPromises()
    expect(button.disabled).toBe(false)
    button.click()
    await flushPromises()
    expect(backend.setProfile).toHaveBeenCalledExactlyOnceWith('support')
    wrapper.unmount()
  })

  // Exigence : EX-016-01
  it('shows the settings reset in the first-launch dialog', async () => {
    const { wrapper } = await mountApp(snapshotOf('reglages-reinitialises'))
    expect(inBody('[data-dialog="first-launch"] [role="alert"]')?.textContent).toContain(
      "Vos réglages n'ont pas pu être vérifiés",
    )
    expect(wrapper.get('[data-banner="settingsReset"]').attributes('data-tone')).toBe('error')
    wrapper.unmount()
  })

  // Exigence : EX-016-01
  it('changes the profile only once the dialog that says its effect is confirmed', async () => {
    const { wrapper, backend } = await mountApp(snapshotOf('sessions'))
    // The radio group of the menu is rendered only once the menu opens (pointer events, see the end-to-end tests):
    // the choice goes through the selector's handler, as the menu's does.
    expect(wrapper.findComponent(DropdownMenuRadioGroup).exists()).toBe(false)
    const selector = wrapper.findComponent({ name: 'ProfileSelector' })
    const vm = selector.vm as unknown as { pick: (value: unknown) => void }
    vm.pick('developpeur')
    await flushPromises()
    expect(inBody('[data-dialog="profile-change"]'), 'the active profile asks nothing').toBeNull()
    vm.pick('support')
    await flushPromises()
    const dialog = inBody('[data-dialog="profile-change"]')
    expect(dialog?.querySelector('h2')?.textContent).toBe('Passer au profil « Support » ?')
    expect(dialog?.textContent).toContain(
      'Les environnements exposés aux clients LLM reviennent aux valeurs par défaut de ce profil : REC, PPROD et PROD.',
    )
    expect(dialog?.textContent).toContain('Environnements sensibles : PPROD et PROD.')
    expect(dialog?.textContent).toContain('Vos durcissements sont conservés.')
    const cancel = [...(dialog?.querySelectorAll('button') ?? [])].find((b) => b.textContent.trim() === 'Annuler')
    cancel?.click()
    await flushPromises()
    expect(backend.setProfile).not.toHaveBeenCalled()
    expect(inBody('[data-dialog="profile-change"]')).toBeNull()

    vm.pick('support')
    await flushPromises()
    inBody('[data-action="confirm-profile"]')?.click()
    await flushPromises()
    expect(backend.setProfile).toHaveBeenCalledExactlyOnceWith('support')
    wrapper.unmount()
  })

  // Exigence : EX-016-01
  it('shows the global banners and their single action', async () => {
    const confirmation = await mountApp(snapshotOf('confirmation'))
    const banner = confirmation.wrapper.get('[data-banner="confirmations"]')
    expect(banner.text()).toContain('Une confirmation attend votre réponse.')
    expect(banner.attributes('data-tone')).toBe('attention')
    expect(banner.findAll('button').map((b) => b.text())).toEqual(['Voir'])
    await banner.get('button').trigger('click')
    expect(confirmation.backend.showConfirmations).toHaveBeenCalledOnce()
    confirmation.wrapper.unmount()

    const suspended = await mountApp(snapshotOf('suspendue'))
    const pause = suspended.wrapper.get('[data-banner="suspended"]')
    expect(pause.attributes('data-tone')).toBe('neutral')
    await pause.get('button').trigger('click')
    expect(suspended.backend.resumeGateway).toHaveBeenCalledOnce()
    suspended.wrapper.unmount()

    const webview = await mountApp(snapshotOf('profil-webview'))
    expect(webview.wrapper.get('[data-banner="startupHardening"]').attributes('role')).toBe('alert')
    webview.wrapper.unmount()

    const calm = await mountApp(snapshotOf('sessions'))
    expect(calm.wrapper.find('[data-banner]').exists()).toBe(false)
    // A refused command is said, and can be dismissed.
    calm.backend.connectSession.mockRejectedValueOnce('session inconnue')
    await calm.wrapper.get('[data-session="sso-int"] button').trigger('click')
    await flushPromises()
    expect(calm.wrapper.get('[data-banner="command"]').text()).toContain('Action impossible : session inconnue.')
    await calm.wrapper.get('[data-banner="command"] button').trigger('click')
    expect(calm.wrapper.find('[data-banner="command"]').exists()).toBe(false)
    calm.wrapper.unmount()
  })

  // Exigence : EX-016-01
  it('shows nothing outside of the application, and says when the state cannot be read', async () => {
    const outside = mount(App, { props: { store: null } })
    expect(outside.get('[data-phase]').text()).toContain("Ouverte ailleurs, elle n'a accès à aucune donnée.")
    const { wrapper, emit } = await mountApp(snapshotOf('sessions'))
    emit(null)
    await flushPromises()
    expect(wrapper.find('header').exists()).toBe(false)
    expect(wrapper.get('[data-phase]').text()).toContain('illisible')
    wrapper.unmount()
  })

  // Exigence : EX-016-03
  it('names every control and labels every region', async () => {
    for (const name of ['sessions', 'connexion-demandee', 'confirmation', 'suspendue', 'premier-lancement'] as const) {
      const { wrapper } = await mountApp(snapshotOf(name))
      for (const button of document.body.querySelectorAll('button')) {
        expect(accessibleName(button), button.outerHTML).not.toBe('')
      }
      expect(wrapper.get('[role="tablist"]').attributes('aria-label')).toBe('Sections')
      for (const section of wrapper.findAll('section')) {
        const id = section.attributes('aria-labelledby')
        expect(id && wrapper.find(`#${id}`).exists()).toBe(true)
      }
      // Card buttons say which session they act on.
      for (const button of wrapper.findAll('[data-session] button')) {
        expect(button.attributes('aria-label')).toMatch(/^(Se connecter à|Déconnecter) SSO /)
      }
      // The state is in words, the dot is decoration.
      for (const dot of wrapper.findAll('[data-dot]')) expect(dot.attributes('aria-hidden')).toBe('true')
      wrapper.unmount()
      document.body.innerHTML = ''
    }
  })

  // Exigence : EX-020-18
  it('offers the restart once an update is in place, and says the sessions will be closed', async () => {
    const { wrapper, backend } = await mountApp(snapshotOf('mise-a-jour-prete'))
    const button = wrapper.get('[data-action="update"]')
    expect(button.text()).toBe('Redémarrer pour mettre à jour')
    expect((button.element as HTMLButtonElement).disabled).toBe(false)
    expect(wrapper.find('[data-update-critical]').exists()).toBe(false)
    expect(accessibleName(button.element)).toBe('Redémarrer pour mettre à jour')
    expect(document.getElementById(button.attributes('aria-describedby') ?? '')?.textContent).toBe(
      'MCP Proxy 0.2.0 est installé : il sera utilisé au prochain démarrage.',
    )

    // Asked first; « Annuler » restarts nothing.
    await button.trigger('click')
    await flushPromises()
    const dialog = inBody('[data-dialog="restart"]')
    expect(dialog?.getAttribute('role')).toBe('alertdialog')
    expect(dialog?.textContent).toContain('Redémarrer pour passer à MCP Proxy 0.2.0 ?')
    expect(dialog?.textContent).toContain('il faudra vous reconnecter')
    const cancel = [...(dialog?.querySelectorAll('button') ?? [])].find((b) => b.textContent.trim() === 'Annuler')
    cancel?.click()
    await flushPromises()
    expect(inBody('[data-dialog="restart"]')).toBeNull()
    expect(backend.restartForUpdate).not.toHaveBeenCalled()

    await button.trigger('click')
    await flushPromises()
    inBody('[data-action="confirm-restart"]')?.click()
    await flushPromises()
    expect(backend.restartForUpdate).toHaveBeenCalledOnce()
    expect(inBody('[data-dialog="restart"]')).toBeNull()
    wrapper.unmount()
  })

  // Exigence : EX-020-18
  it('says when a critical update will restart by itself', async () => {
    const { wrapper } = await mountApp(snapshotOf('mise-a-jour-critique'))
    expect(wrapper.get('[data-update-critical]').text()).toBe('Mise à jour critique')
    expect(wrapper.get('#update-help').text()).toContain('il redémarrera de lui-même dès que plus aucun appel')
    await wrapper.get('[data-action="update"]').trigger('click')
    await flushPromises()
    expect(inBody('[data-dialog="restart"]')?.textContent).toContain('il redémarrera de lui-même')
    wrapper.unmount()
  })

  // Exigence : EX-020-18
  it('shows the reason when the restart is refused', async () => {
    const { wrapper, backend } = await mountApp(snapshotOf('mise-a-jour-prete'))
    backend.restartForUpdate.mockRejectedValueOnce('aucune mise à jour prête à appliquer')
    await wrapper.get('[data-action="update"]').trigger('click')
    await flushPromises()
    inBody('[data-action="confirm-restart"]')?.click()
    await flushPromises()
    expect(wrapper.text()).toContain('Action impossible : aucune mise à jour prête à appliquer.')
    wrapper.unmount()
  })

  // Exigence : EX-023-15
  it('the menu of the header opens « Clients LLM… »', async () => {
    const { wrapper, backend } = await mountApp(snapshotOf('sessions'))
    const trigger = wrapper.get('[data-action="app-menu"]')
    expect(trigger.attributes('aria-label')).toBe('Menu')
    // The menu opens on pointer events (see the end-to-end tests): its entry sets what it opens.
    const menu = wrapper.findComponent({ name: 'AppMenu' })
    ;(menu.vm as unknown as { clients: boolean }).clients = true
    await flushPromises()
    expect(inBody('[data-dialog="clients"]')?.textContent).toContain('Clients LLM')
    expect(backend.getLlmClients).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  // Exigence : EX-024-12
  it('the menu of the header opens « Auto-diagnostic », which runs the checks', async () => {
    const { wrapper, backend } = await mountApp(snapshotOf('sessions'))
    const menu = wrapper.findComponent({ name: 'AppMenu' })
    expect(backend.runDoctor).not.toHaveBeenCalled()
    ;(menu.vm as unknown as { doctor: boolean }).doctor = true
    await flushPromises()
    expect(inBody('[data-dialog="doctor"]')?.textContent).toContain('Auto-diagnostic')
    expect(backend.runDoctor).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })
})
