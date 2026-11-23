import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createMockBackend } from '@/api/mock'
import type { Backend } from '@/api/backend'
import { STORE, createStore } from '@/store/snapshot'
import { NOW, clientsFixture, previewFixture, snapshotOf, spyBackend } from '@/test/fixtures'
import ClientsDialog from './ClientsDialog.vue'
import { diffLines } from './model'

function mountDialog(backend: Backend) {
  const store = createStore(backend)
  const wrapper = mount(ClientsDialog, {
    props: { open: true },
    global: { provide: { [STORE as symbol]: store } },
    attachTo: document.body,
  })
  return wrapper
}

const $ = (selector: string) => document.body.querySelector<HTMLElement>(selector)
const $$ = (selector: string) => [...document.body.querySelectorAll<HTMLElement>(selector)]
const row = (id: string) => $(`[data-location="${id}"]`)
const text = () => $('[data-dialog="clients"]')?.textContent ?? ''

async function click(element: HTMLElement | null | undefined) {
  if (!element) throw new Error('missing element')
  element.click()
  await flushPromises()
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('dialog « Clients LLM »', () => {
  // Exigence : EX-023-15
  it('lists each location with its state, and offers only what makes sense', async () => {
    const spy = spyBackend(snapshotOf('sessions'))
    const wrapper = mountDialog(spy.backend)
    await flushPromises()
    expect($$('[data-location]').map((e) => [e.dataset.location, e.dataset.state])).toEqual([
      ['claude-desktop', 'configured'],
      ['claude-desktop-msix', 'otherCommand'],
      ['vscode', 'unreadable'],
    ])
    expect(row('claude-desktop')?.querySelector('[data-client-state]')?.textContent).toBe('configuré')
    expect(row('claude-desktop-msix')?.textContent).toContain('configuré avec un autre chemin')
    expect(row('claude-desktop-msix')?.textContent).toContain(
      "L'entrée « mcp-proxy » lance C:\\Users\\exemple\\Downloads\\mcp-proxy.exe. « Installer » la remplace",
    )
    expect(row('vscode')?.textContent).toContain('JSON invalide à la ligne 3. MCP Proxy n\'écrit rien')
    // Orange for the entry that waits for « Installer », red for the file that cannot be read.
    expect(row('claude-desktop-msix')?.querySelector('[data-client-state]')?.className).toContain('text-attention-text')
    expect(row('vscode')?.querySelector('[data-client-state]')?.className).toContain('text-destructive-text')
    const install = (id: string) => row(id)?.querySelector<HTMLButtonElement>('[data-action="client-install"]')
    const remove = (id: string) => row(id)?.querySelector<HTMLButtonElement>('[data-action="client-remove"]')
    expect(install('claude-desktop')?.disabled).toBe(true)
    expect(remove('claude-desktop')?.disabled).toBe(false)
    expect(install('claude-desktop-msix')?.disabled).toBe(false)
    expect(install('vscode')?.disabled).toBe(false)
    expect(install('vscode')?.getAttribute('aria-label')).toBe('Installer MCP Proxy dans VS Code')
    // Both files of Claude Desktop are there: the dialog says that only one is read.
    expect($('[data-notice="two-locations"]')).not.toBeNull()
    expect(row('claude-desktop')?.textContent).toContain('Sans doute pas lu')
    expect(text()).toContain('C:\\Users\\exemple\\AppData\\Local\\Programs\\MCP Proxy\\mcp-proxy.exe stdio')
    wrapper.unmount()
  })

  // Exigence : EX-023-15
  it('without MCP Proxy installed, says so and offers no « Installer » nor copy', async () => {
    const spy = spyBackend(snapshotOf('sessions'))
    spy.backend.getLlmClients.mockResolvedValue(clientsFixture('notInstalled'))
    const wrapper = mountDialog(spy.backend)
    await flushPromises()
    expect($('[data-notice="not-installed"]')?.textContent).toContain("MCP Proxy n'est pas installé")
    expect($('[data-action="client-copy"]')).toBeNull()
    // Clients not installed have no buttons at all.
    expect($$('[data-action="client-install"]')).toEqual([])
    wrapper.unmount()
  })

  // Exigence : EX-023-15
  it('previews, then sends the action with the fingerprint of the preview, then says to restart', async () => {
    const spy = spyBackend(snapshotOf('sessions'))
    spy.backend.previewLlmClient.mockResolvedValue(previewFixture(2))
    spy.backend.applyLlmClient.mockResolvedValue({
      status: 'applied',
      backup: 'claude_desktop_config.json.bak-20261122-140307',
    })
    const wrapper = mountDialog(spy.backend)
    await flushPromises()
    await click(row('claude-desktop-msix')?.querySelector('[data-action="client-install"]'))
    expect(spy.backend.previewLlmClient.mock.calls).toEqual([['claude-desktop-msix', 'install']])
    expect(text()).toContain('Ajouter MCP Proxy à Claude Desktop (paquet MSIX) ?')
    expect(text()).toContain("L'entrée « mcp-proxy » actuelle est remplacée.")
    expect(text()).toContain("Une copie du fichier actuel est enregistrée à côté de lui avant l'écriture.")
    const signs = $$('[data-diff] [data-sign]').map((e) => e.dataset.sign)
    expect(signs[0]).toBe('-')
    expect(signs.slice(1).every((s) => s === '+')).toBe(true)
    expect($('[data-diff]')?.textContent).toContain('- "mcp-proxy": { "command": "C:\\\\Users\\\\exemple\\\\Downloads')
    expect($('[data-diff]')?.textContent).toContain('autres champs : args, env')
    expect(spy.backend.applyLlmClient).not.toHaveBeenCalled()
    await click($('[data-action="client-confirm"]'))
    expect(spy.backend.applyLlmClient.mock.calls).toEqual([
      ['claude-desktop-msix', 'install', previewFixture(2).fingerprint],
    ])
    expect(text()).toContain('MCP Proxy a été ajouté à Claude Desktop (paquet MSIX).')
    expect(text()).toContain('claude_desktop_config.json.bak-20261122-140307')
    expect($('[data-notice="restart"]')?.textContent).toContain('zone de notification')
    expect($('[data-notice="restart"]')?.textContent).toContain('« Quit »')
    expect(spy.backend.getLlmClients).toHaveBeenCalledTimes(2)
    wrapper.unmount()
  })

  // Exigence : EX-023-15
  it('shows a new preview when the file changed since, and applies only that one', async () => {
    const backend = createMockBackend('clients-modifie', () => NOW)
    const apply = vi.spyOn(backend, 'applyLlmClient')
    const wrapper = mountDialog(backend)
    await flushPromises()
    await click(row('vscode')?.querySelector('[data-action="client-install"]'))
    expect(text()).toContain("L'entrée « mcp-proxy » est ajoutée.")
    await click($('[data-action="client-confirm"]'))
    expect($('[data-notice="changed"]')?.textContent).toContain("Le fichier a changé depuis l'aperçu")
    expect($('[data-dialog="clients"]')?.dataset.phase).toBe('preview')
    await click($('[data-action="client-confirm"]'))
    expect(apply).toHaveBeenCalledTimes(2)
    expect(apply.mock.calls[0]?.[2]).not.toBe(apply.mock.calls[1]?.[2])
    expect(text()).toContain('MCP Proxy a été ajouté à VS Code.')
    expect($('[data-notice="restart"]')?.textContent).toContain('Reload Window')
    await click($('[data-action="client-back"]'))
    expect(row('vscode')?.dataset.state).toBe('configured')
    wrapper.unmount()
  })

  // Exigence : EX-023-15
  it('for a file that cannot be read safely, writes nothing and gives the entry to paste', async () => {
    const backend = createMockBackend('clients-illisible', () => NOW)
    const apply = vi.spyOn(backend, 'applyLlmClient')
    const writeText = vi.fn(() => Promise.resolve())
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    const wrapper = mountDialog(backend)
    await flushPromises()
    await click(row('claude-desktop')?.querySelector('[data-action="client-install"]'))
    expect(text()).toContain('Ce fichier ne peut pas être modifié sans risque (JSON invalide à la ligne 12)')
    expect($('[data-action="client-confirm"]')).toBeNull()
    expect($('[data-snippet]')?.textContent).toContain('"mcpServers"')
    await click($('[data-action="client-copy-entry"]'))
    expect(writeText).toHaveBeenCalledWith($('[data-snippet]')?.textContent)
    expect($('[data-copy]')?.textContent).toContain('Configuration copiée')
    await click(row('claude-desktop')?.querySelector('[data-action="client-remove"]') ?? $('[data-action="client-back"]'))
    expect(apply).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  // Exigence : EX-023-15
  it('« Copier la configuration » copies the generic entry, or shows it when the clipboard refuses', async () => {
    const spy = spyBackend(snapshotOf('sessions'))
    const writeText = vi.fn(() => Promise.reject(new Error('refusé')))
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    const wrapper = mountDialog(spy.backend)
    await flushPromises()
    await click($('[data-action="client-copy"]'))
    expect(writeText).toHaveBeenCalledWith(clientsFixture().snippet)
    expect($('[data-copy]')?.textContent).toContain('Copie impossible')
    expect($('[data-copied]')?.textContent).toBe(clientsFixture().snippet)
    wrapper.unmount()
  })

  // Exigence : EX-023-15
  it('shows the refusal of the application in red, in the dialog', async () => {
    const spy = spyBackend(snapshotOf('sessions'))
    spy.backend.previewLlmClient.mockRejectedValue("VS Code n'est pas installé sur ce poste : rien n'est créé")
    const wrapper = mountDialog(spy.backend)
    await flushPromises()
    await click(row('vscode')?.querySelector('[data-action="client-install"]'))
    expect($('[data-error]')?.textContent).toContain("VS Code n'est pas installé sur ce poste")
    expect($('[data-error]')?.className).toContain('text-destructive-text')
    expect($('[data-dialog="clients"]')?.dataset.phase).toBe('list')
    wrapper.unmount()
  })

  // Exigence : EX-023-15
  it('a removal shows the entry that goes, and nothing added', () => {
    const lines = diffLines(previewFixture(1))
    expect(lines).toEqual([
      {
        sign: '-',
        text: '"mcp-proxy": { "command": "C:\\\\Users\\\\exemple\\\\AppData\\\\Local\\\\Programs\\\\MCP Proxy\\\\mcp-proxy.exe", … (autres champs : args) }',
      },
    ])
    expect(diffLines(previewFixture(0))).toEqual([])
  })
})
