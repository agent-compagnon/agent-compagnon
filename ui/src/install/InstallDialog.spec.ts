import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import InstallDialog from './InstallDialog.vue'
import { type InstallAnswer, type InstallData, installData, sender } from './install'
import { scenarioOf, simulated } from './mock'

const INSTALL: InstallData = {
  kind: 'install',
  version: '0.2.0',
  installedVersion: null,
  autostart: false,
  location: '%LOCALAPPDATA%\\Programs\\MCP Proxy',
  command: 'answer_install',
}
const UPDATE: InstallData = { ...INSTALL, kind: 'update', installedVersion: '0.1.0' }

type SendMock = ReturnType<typeof vi.fn<(answer: InstallAnswer) => Promise<void>>>

function mountWith(data: InstallData, send: SendMock) {
  return mount(InstallDialog, { props: { data, send } })
}

describe('installation window', () => {
  // Exigence : EX-019-13
  it('installs with the autostart box as the user left it', async () => {
    const send: SendMock = vi.fn(() => new Promise<void>(() => undefined))
    const wrapper = mountWith(INSTALL, send)
    expect(wrapper.find('h1').text()).toBe('Installer MCP Proxy')
    expect(wrapper.text()).toContain("MCP Proxy 0.2.0 s'installe pour votre compte Windows, sans droits d'administrateur.")
    expect(wrapper.find('.location').text()).toBe('%LOCALAPPDATA%\\Programs\\MCP Proxy')
    const box = wrapper.find('input[name="autostart"]')
    expect(wrapper.text()).toContain('Démarrer MCP Proxy avec Windows')
    expect((box.element as HTMLInputElement).checked).toBe(false)
    await box.setValue(true)
    await wrapper.find('button.confirm').trigger('click')
    expect(send).toHaveBeenCalledExactlyOnceWith({ choice: 'install', autostart: true })
    expect(wrapper.find('.working').text()).toBe('Installation en cours…')
    for (const button of wrapper.findAll('button')) expect(button.attributes('disabled')).toBeDefined()
    expect(box.attributes('disabled')).toBeDefined()
    await wrapper.find('button.confirm').trigger('click')
    expect(send).toHaveBeenCalledTimes(1)
  })

  // Exigence : EX-019-13
  it('starts with the box checked when Windows already starts MCP Proxy', () => {
    const wrapper = mountWith({ ...INSTALL, autostart: true }, vi.fn())
    expect((wrapper.find('input[name="autostart"]').element as HTMLInputElement).checked).toBe(true)
  })

  // Exigence : EX-019-13
  it('can be left for later', async () => {
    const send: SendMock = vi.fn(() => Promise.resolve())
    const wrapper = mountWith(INSTALL, send)
    const later = wrapper.find('button.later')
    expect(later.text()).toBe('Pas maintenant')
    await later.trigger('click')
    expect(send).toHaveBeenCalledExactlyOnceWith({ choice: 'later' })
  })

  // Exigence : EX-019-13
  it('offers to update the installation and to launch the installed version instead', async () => {
    const send: SendMock = vi.fn(() => new Promise<void>(() => undefined))
    const wrapper = mountWith(UPDATE, send)
    expect(wrapper.find('h1').text()).toBe('Mettre à jour MCP Proxy')
    expect(wrapper.find('.versions').text()).toBe('La version 0.1.0 est installée. Cet exe est la version 0.2.0.')
    expect(wrapper.text()).toContain('les sessions ouvertes devront être reconnectées')
    expect(wrapper.find('input[name="autostart"]').exists()).toBe(false)
    expect(wrapper.find('button.later').text()).toBe('Lancer la version 0.1.0')
    await wrapper.find('button.confirm').trigger('click')
    expect(send).toHaveBeenCalledExactlyOnceWith({ choice: 'update' })
    expect(wrapper.find('.working').text()).toBe('Mise à jour en cours…')
  })

  // Exigence : EX-019-13
  it('says why it failed, then continues from here or quits', async () => {
    const reason = "Création du dossier d'installation impossible (accès refusé). Rien n'a été modifié."
    const send: SendMock = vi.fn((answer: InstallAnswer) =>
      answer.choice === 'install' ? Promise.reject(new Error(reason)) : Promise.resolve(),
    )
    const wrapper = mountWith(INSTALL, send)
    await wrapper.find('button.confirm').trigger('click')
    await flushPromises()
    expect(wrapper.find('main').attributes('data-state')).toBe('failed')
    expect(wrapper.find('h1').text()).toBe("L'installation n'a pas abouti")
    expect(wrapper.find('[role="alert"]').text()).toBe(reason)
    expect(wrapper.text()).toContain("MCP Proxy peut tout de même fonctionner depuis l'emplacement de cet exe")
    expect(wrapper.find('button.quit').text()).toBe('Quitter')
    await wrapper.find('button.continue').trigger('click')
    expect(send).toHaveBeenLastCalledWith({ choice: 'continue' })

    const quitting = mountWith(UPDATE, vi.fn((answer: InstallAnswer) =>
      answer.choice === 'update' ? Promise.reject(new Error('refus')) : Promise.resolve(),
    ))
    await quitting.find('button.confirm').trigger('click')
    await flushPromises()
    expect(quitting.find('h1').text()).toBe("La mise à jour n'a pas abouti")
    await quitting.find('button.quit').trigger('click')
  })

  // Exigence : EX-019-13
  it('reads its data from the application and sends through its command only', async () => {
    expect(installData({})).toBeNull()
    expect(installData({ __MCP_PROXY_INSTALL__: { ...INSTALL, kind: 'uninstall' } })).toBeNull()
    expect(installData({ __MCP_PROXY_INSTALL__: { ...INSTALL, installedVersion: '0.1.0' } })).toBeNull()
    expect(installData({ __MCP_PROXY_INSTALL__: { ...UPDATE, installedVersion: null } })).toBeNull()
    expect(installData({ __MCP_PROXY_INSTALL__: { ...INSTALL, autostart: 'oui' } })).toBeNull()
    expect(installData({ __MCP_PROXY_INSTALL__: INSTALL })).toEqual(INSTALL)
    expect(installData({ __MCP_PROXY_INSTALL__: UPDATE })).toEqual(UPDATE)

    const invoke = vi.fn<(cmd: string, args: Record<string, unknown>) => Promise<unknown>>(() =>
      Promise.reject(new Error('Copie de l\'exe impossible (disque plein).')),
    )
    const send = sender({ __TAURI_INTERNALS__: { invoke } }, INSTALL.command)
    await expect(send({ choice: 'install', autostart: false })).rejects.toThrow('disque plein')
    expect(invoke).toHaveBeenCalledExactlyOnceWith('answer_install', {
      answer: { choice: 'install', autostart: false },
    })
    await expect(sender({}, INSTALL.command)({ choice: 'quit' })).rejects.toThrow('MCP Proxy ne répond pas')
  })

  // Exigence : EX-019-13
  it('simulates the application by scenario', async () => {
    expect(scenarioOf('')).toBe('installation')
    expect(scenarioOf('?scenario=inconnu')).toBe('installation')
    expect(simulated('?scenario=mise-a-jour').data.installedVersion).toBe('0.1.0')
    vi.useFakeTimers()
    try {
      const failing = simulated('?scenario=installation-echec').send({ choice: 'install', autostart: true })
      const assertion = expect(failing).rejects.toThrow('accès refusé')
      await vi.runAllTimersAsync()
      await assertion
    } finally {
      vi.useRealTimers()
    }
  })
})
