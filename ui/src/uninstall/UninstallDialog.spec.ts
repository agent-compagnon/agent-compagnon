import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import UninstallDialog from './UninstallDialog.vue'
import example from './uninstall.example.json'
import {
  type UninstallAnswer,
  type UninstallData,
  type UninstallReport,
  isData,
  isReport,
  sender,
  uninstallData,
} from './uninstall'
import { scenarioOf, simulated } from './mock'

function data(): UninstallData {
  if (!isData(example.data)) throw new Error('contract')
  return example.data
}

function report(which: 'complete' | 'withLeftovers'): UninstallReport {
  const value: unknown = example[which]
  if (!isReport(value)) throw new Error('contract')
  return value
}

type SendMock = ReturnType<typeof vi.fn<(answer: UninstallAnswer) => Promise<UninstallReport | null>>>

function mountWith(value: UninstallData, send: SendMock) {
  return mount(UninstallDialog, { props: { data: value, send } })
}

describe('uninstallation window', () => {
  // Exigence : EX-022-12
  it('reads exactly what the application writes', () => {
    expect(isData(example.data)).toBe(true)
    expect(isReport(example.complete)).toBe(true)
    expect(isReport(example.withLeftovers)).toBe(true)
    expect(isData({ ...example.data, path: 'C:\\' })).toBe(false)
    expect(isData({ ...example.data, clients: [{ ...example.data.clients[0], entry: 'mine' }] })).toBe(false)
    expect(isReport({ ...example.complete, keptInstalled: 'non' })).toBe(false)
    expect(isReport({ ...example.complete, left: [{ what: 'x', why: 'y' }] })).toBe(false)
    expect(uninstallData({})).toBeNull()
    expect(uninstallData({ __MCP_PROXY_UNINSTALL__: example.data })).toEqual(example.data)
  })

  // Exigence : EX-022-11
  it('says what goes and what stays, then uninstalls with the box ticked', async () => {
    const send: SendMock = vi.fn(() => new Promise<UninstallReport | null>(() => undefined))
    const wrapper = mountWith(data(), send)
    expect(wrapper.find('h1').text()).toBe('Désinstaller MCP Proxy')
    const removed = wrapper.find('.removed').text()
    expect(removed).toContain('%LOCALAPPDATA%\\Programs\\MCP Proxy')
    expect(removed).toContain('les journaux, les réglages')
    expect(removed).toContain('%LOCALAPPDATA%\\MCP Proxy')
    const box = wrapper.find('input[name="removeClients"]')
    expect((box.element as HTMLInputElement).checked).toBe(true)
    expect(wrapper.text()).toContain('Retirer MCP Proxy des clients LLM')
    expect(wrapper.find('#clients-hint').text()).toContain('de Claude Desktop, rien d\'autre')
    const kept = wrapper.find('.kept').text()
    expect(kept).toContain("l'entrée « mcp-proxy » de VS Code, qui lance un autre programme (C:\\Outils\\mcp-proxy-dev.exe)")
    expect(kept).toContain('Claude Desktop (paquet MSIX), illisible (JSON invalide à la ligne 12)')
    expect(kept).toContain('.bak-')
    expect(kept).toContain("aucun mot de passe ni jeton")
    await wrapper.find('button.confirm').trigger('click')
    expect(send).toHaveBeenCalledExactlyOnceWith({ choice: 'uninstall', removeClients: true })
    expect(wrapper.find('.working').text()).toBe('Désinstallation en cours…')
    for (const button of wrapper.findAll('button')) expect(button.attributes('disabled')).toBeDefined()
    expect(box.attributes('disabled')).toBeDefined()
    await wrapper.find('button.confirm').trigger('click')
    expect(send).toHaveBeenCalledTimes(1)
  })

  // Exigence : EX-022-11
  it('leaves the clients when the box is unticked, and says so', async () => {
    const send: SendMock = vi.fn(() => new Promise<UninstallReport | null>(() => undefined))
    const wrapper = mountWith(data(), send)
    await wrapper.find('input[name="removeClients"]').setValue(false)
    expect(wrapper.find('#clients-hint').text()).toBe(
      "Les fichiers des clients LLM ne sont pas modifiés : l'entrée « mcp-proxy » de Claude Desktop reste, et ne lancera plus rien.",
    )
    await wrapper.find('button.confirm').trigger('click')
    expect(send).toHaveBeenCalledExactlyOnceWith({ choice: 'uninstall', removeClients: false })
    const none = mountWith({ ...data(), clients: [] }, vi.fn())
    expect(none.find('#clients-hint').text()).toBe("Aucun client LLM n'est configuré avec MCP Proxy sur ce compte.")
  })

  // Exigence : EX-022-11
  it('shows the report: everything gone, or what stays and why', async () => {
    const send: SendMock = vi.fn(() => Promise.resolve(report('complete')))
    const wrapper = mountWith(data(), send)
    await wrapper.find('button.confirm').trigger('click')
    await flushPromises()
    expect(wrapper.find('main').attributes('data-state')).toBe('reported')
    expect(wrapper.find('h1').text()).toBe('MCP Proxy est désinstallé')
    expect(wrapper.find('.done-removed').text()).toContain('Raccourci « MCP Proxy » du menu Démarrer')
    expect(wrapper.find('.done-removed').text()).toContain('Programme mcp-proxy.exe : à la fermeture de cette fenêtre')
    expect(wrapper.find('.done-left').exists()).toBe(false)
    await wrapper.find('button.close').trigger('click')
    expect(send).toHaveBeenLastCalledWith({ choice: 'close' })

    const partial = mountWith(data(), vi.fn(() => Promise.resolve(report('withLeftovers'))))
    await partial.find('button.confirm').trigger('click')
    await flushPromises()
    expect(partial.find('h1').text()).toBe('Désinstallation incomplète')
    const failed = partial.findAll('.done-left li.failed')
    expect(failed).toHaveLength(1)
    expect(failed[0]?.text()).toContain('fichier utilisé par un autre programme')
    expect(partial.find('.kept-installed').text()).toContain('relancez la désinstallation')
  })

  // Exigence : EX-022-11
  it('says why nothing was done, then tries again or closes', async () => {
    const reason = example.notStopped
    let calls = 0
    const send: SendMock = vi.fn((answer: UninstallAnswer) => {
      calls += 1
      if (answer.choice !== 'uninstall') return Promise.resolve(null)
      return calls === 1 ? Promise.reject(new Error(reason)) : Promise.resolve(report('complete'))
    })
    const wrapper = mountWith(data(), send)
    await wrapper.find('button.confirm').trigger('click')
    await flushPromises()
    expect(wrapper.find('h1').text()).toBe("La désinstallation n'a pas eu lieu")
    expect(wrapper.find('[role="alert"]').text()).toBe(reason)
    await wrapper.find('button.retry').trigger('click')
    await flushPromises()
    expect(send).toHaveBeenLastCalledWith({ choice: 'uninstall', removeClients: true })
    expect(wrapper.find('main').attributes('data-state')).toBe('reported')
  })

  // Exigence : EX-022-11
  it('sends through its command only, and reads the report strictly', async () => {
    const invoke = vi.fn<(cmd: string, args: Record<string, unknown>) => Promise<unknown>>(() =>
      Promise.resolve(example.complete),
    )
    const send = sender({ __TAURI_INTERNALS__: { invoke } }, 'answer_uninstall')
    await expect(send({ choice: 'uninstall', removeClients: true })).resolves.toEqual(example.complete)
    expect(invoke).toHaveBeenCalledExactlyOnceWith('answer_uninstall', {
      answer: { choice: 'uninstall', removeClients: true },
    })
    invoke.mockImplementationOnce(() => Promise.resolve({ removed: 'tout' }))
    await expect(send({ choice: 'uninstall', removeClients: true })).rejects.toThrow('compte rendu illisible')
    invoke.mockImplementationOnce(() => Promise.resolve(null))
    await expect(send({ choice: 'cancel' })).resolves.toBeNull()
    await expect(sender({}, 'answer_uninstall')({ choice: 'cancel' })).rejects.toThrow('MCP Proxy ne répond pas')
  })

  // Exigence : EX-022-11
  it('simulates the application by scenario', async () => {
    expect(scenarioOf('')).toBe('desinstallation')
    expect(scenarioOf('?scenario=inconnu')).toBe('desinstallation')
    expect(simulated('?scenario=desinstallation-sans-client').data.clients).toEqual([])
    vi.useFakeTimers()
    try {
      const failing = simulated('?scenario=desinstallation-echec').send({ choice: 'uninstall', removeClients: true })
      const assertion = expect(failing).rejects.toThrow("n'a pas pu être fermé")
      await vi.runAllTimersAsync()
      await assertion
      const unticked = simulated('').send({ choice: 'uninstall', removeClients: false })
      await vi.runAllTimersAsync()
      const result = await unticked
      expect(result?.removed.some((r) => r.startsWith('Entrée « mcp-proxy »'))).toBe(false)
      expect(result?.left[0]?.why).toBe('laissée, comme demandé')
    } finally {
      vi.useRealTimers()
    }
  })
})
