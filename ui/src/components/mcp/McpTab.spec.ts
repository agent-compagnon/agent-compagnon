import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import type { AppSnapshot } from '@/api/types'
import { STORE, createStore } from '@/store/snapshot'
import { inTooltips, snapshotOf, spyBackend } from '@/test/fixtures'
import McpTab from './McpTab.vue'

function mountTab(snapshot: AppSnapshot) {
  const spy = spyBackend(snapshot)
  const wrapper = mount(inTooltips(McpTab, { snapshot }), {
    global: { provide: { [STORE as symbol]: createStore(spy.backend) } },
    attachTo: document.body,
  })
  return { wrapper, backend: spy.backend }
}

describe('MCP tab', () => {
  // Exigence : EX-017-04
  it('a pastille exposes or withdraws its environment; a sensitive one is confirmed first', async () => {
    const { wrapper, backend } = mountTab(snapshotOf('sessions'))
    await wrapper.get('[data-environment="dev"]').trigger('click')
    await wrapper.get('[data-environment="qua"]').trigger('click')
    expect(backend.setEnvironmentExposed.mock.calls).toEqual([
      ['dev', false],
      ['qua', true],
    ])
    await wrapper.get('[data-environment="prod"]').trigger('click')
    await flushPromises()
    expect(backend.setEnvironmentExposed).toHaveBeenCalledTimes(2)
    const confirm = document.querySelector<HTMLElement>('[data-action="confirm-exposure"]')
    expect(document.body.textContent).toContain('Exposer PROD aux clients LLM ?')
    confirm?.click()
    await flushPromises()
    expect(backend.setEnvironmentExposed.mock.calls[2]).toEqual(['prod', true])
    wrapper.unmount()
  })

  // Exigence : EX-017-04
  it('a switch cuts its MCP; it is disabled, and says why, when its environment is not exposed', async () => {
    const { wrapper, backend } = mountTab(snapshotOf('sessions'))
    const dev = wrapper.get('[data-row="dev/kibana-logs"]')
    expect(dev.get('[data-action="expose"]').attributes('disabled')).toBeUndefined()
    await dev.get('[data-action="expose"]').trigger('click')
    expect(backend.setMcpExposed.mock.calls).toEqual([['kibana-logs', false]])
    expect(dev.get('[data-field="why"]').text()).toBe(
      'Couper Kibana Logs retire ses outils dans tous les environnements.',
    )
    const prod = wrapper.get('[data-row="prod/kibana-logs"]')
    expect(prod.get('[data-action="expose"]').attributes('disabled')).toBeDefined()
    expect(prod.get('[data-field="why"]').text()).toBe("PROD n'est pas exposé : exposez-le d'abord avec sa pastille.")
    await prod.get('[data-action="expose"]').trigger('click')
    expect(backend.setMcpExposed).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  // Exigence : EX-017-05
  it('shows the update banner; « Voir » filters on the MCP, « J\'ai vu » acknowledges it', async () => {
    const { wrapper, backend } = mountTab(snapshotOf('mcp-mis-a-jour'))
    const banner = wrapper.get('[data-update="rec/kibana-logs"]')
    expect(banner.text()).toContain('Kibana Logs (REC) a été mis à jour : 2 nouveaux outils et 1 outil modifié.')
    expect(banner.attributes('data-tone')).toBe('neutral')
    await banner.get('[data-action="see"]').trigger('click')
    expect(wrapper.findAll('[data-row]').map((r) => r.attributes('data-row'))).toEqual(['rec/kibana-logs'])
    const tools = wrapper.get('[data-tools="rec/kibana-logs"]')
    expect(tools.findAll('[data-mark="added"]')).toHaveLength(2)
    expect(tools.get('[data-tool="export_logs"] [data-field="tool-policy"]').text()).toBe(
      'confirmation par notification',
    )
    expect(tools.find('[data-tool="search_logs"] [data-mark="modified"]').exists()).toBe(true)
    await banner.get('[data-action="seen"]').trigger('click')
    expect(backend.acknowledgeMcpUpdate.mock.calls).toEqual([['rec', 'kibana-logs']])
    await wrapper.get('[data-action="clear-focus"]').trigger('click')
    expect(wrapper.findAll('[data-row]')).toHaveLength(24)
    expect(mountTab(snapshotOf('sessions')).wrapper.find('[data-banner="mcp-update"]').exists()).toBe(false)
    wrapper.unmount()
  })

  // Exigence : EX-017-06
  it('turns the tool counter orange above the threshold only', () => {
    const below = mountTab(snapshotOf('sessions')).wrapper.get('[data-counter]')
    expect(below.attributes('data-over')).toBe('false')
    expect(below.classes().some((c) => c.includes('attention'))).toBe(false)
    const over = mountTab(snapshotOf('seuil')).wrapper
    const counter = over.get('[data-counter]')
    expect(counter.text()).toBe('106 outils exposés')
    expect(counter.attributes('data-over')).toBe('true')
    expect(counter.classes()).toContain('text-attention-text')
    expect(over.get('#tool-counter-help').text()).toContain('certains clients LLM plafonnent à 128 outils')
  })

  // Exigence : EX-017-02
  it('unfolds and highlights the matching tools, and never renders a description as HTML', async () => {
    const snapshot = snapshotOf('sessions')
    const tool = snapshot.mcps[0]?.instances[0]?.tools[0]
    if (tool) tool.description = '<img src=x onerror="alert(1)"> deletes an index <b>now</b>'
    const { wrapper } = mountTab(snapshot)
    await wrapper.get('[data-field="search"]').setValue('kibána INDEX')
    const tools = wrapper.get('[data-tools="dev/kibana-logs"]')
    expect(tools.get('[data-field="matching"]').text()).toBe('2 outils sur 4 correspondent à la recherche.')
    expect(tools.findAll('[data-highlight]').map((m) => m.text())).toContain('index')
    expect(tools.find('img').exists()).toBe(false)
    expect(tools.find('b').exists()).toBe(false)
    expect(tools.text()).toContain('<img src=x onerror="alert(1)">')
    await wrapper.get('[data-field="search"]').setValue('introuvable')
    expect(wrapper.get('[data-empty="rows"]').text()).toContain('Aucun MCP ne correspond à « introuvable ».')
    wrapper.unmount()
  })

  // Exigence : EX-017-03
  it('says so when no environment is exposed, and still lists every MCP', () => {
    const { wrapper } = mountTab(snapshotOf('rien-expose'))
    expect(wrapper.get('[data-empty="exposed"]').text()).toContain("Aucun environnement n'est exposé")
    expect(wrapper.findAll('[data-row]')).toHaveLength(24)
    expect(wrapper.get('[data-counter]').text()).toBe('0 outil exposé')
  })
})
