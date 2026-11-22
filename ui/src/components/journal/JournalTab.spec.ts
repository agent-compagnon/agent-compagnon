import { type VueWrapper, flushPromises, mount } from '@vue/test-utils'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { exampleEntries } from '@/api/mock-journal'
import type { AppSnapshot, JournalEntry } from '@/api/types'
import McpTab from '@/components/mcp/McpTab.vue'
import { STORE, createStore } from '@/store/snapshot'
import { inTooltips, snapshotOf, spyBackend } from '@/test/fixtures'
import JournalTab from './JournalTab.vue'

// jsdom lays nothing out: the list is given a height, so that the virtualizer renders the rows of its view.
beforeAll(() => {
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(2000)
})
afterAll(() => {
  vi.restoreAllMocks()
})

async function mountTab(entries: JournalEntry[] = exampleEntries(), snapshot: AppSnapshot = snapshotOf('sessions')) {
  const spy = spyBackend(snapshot, entries)
  const store = createStore(spy.backend)
  await store.journal.start()
  const wrapper = mount(inTooltips(JournalTab, { snapshot }), {
    global: { provide: { [STORE as symbol]: store } },
    attachTo: document.body,
  })
  await flushPromises()
  return { wrapper, store, ...spy }
}

function shownSeqs(wrapper: VueWrapper): number[] {
  return wrapper.findAll('[data-entry]').map((e) => Number(e.attributes('data-entry')))
}

describe('Journal tab', () => {
  // Exigence : EX-018-03
  it('unfolds a row in error with what the core kept; a normal row has nothing to unfold', async () => {
    const { wrapper } = await mountTab()
    // Newest first.
    expect(shownSeqs(wrapper).slice(0, 3)).toEqual([10, 9, 8])
    const normal = wrapper.get('[data-entry="1"]')
    expect(normal.attributes('data-expandable')).toBe('false')
    expect(normal.find('[data-action="toggle"]').exists()).toBe(false)
    await normal.trigger('click')
    expect(normal.find('[data-details]').exists()).toBe(false)

    const error = wrapper.get('[data-entry="2"]')
    expect(error.attributes('data-error')).toBe('true')
    const toggle = error.get('[data-action="toggle"]')
    expect(toggle.attributes('aria-expanded')).toBe('false')
    await toggle.trigger('click')
    expect(toggle.attributes('aria-expanded')).toBe('true')
    const details = error.get('[data-details]')
    expect(details.get('[data-detail="trace"]').text()).toBe('42')
    expect(details.get('[data-detail="client"]').text()).toBe('Claude Desktop')
    expect(details.get('[data-detail="session"]').text()).toBe('sso-rec')
    expect(details.get('[data-detail="policy"]').text()).toBe('autorisé — règle « read » de la racine')
    expect(details.get('[data-detail="request"]').text()).toBe(
      JSON.stringify({ job: 88121, project: 'paiement/api' }, null, 2),
    )
    expect(details.get('[data-detail="response"]').text()).toContain('"text": "HTTP 502 Bad Gateway"')
    await toggle.trigger('click')
    expect(error.find('[data-details]').exists()).toBe(false)

    // Denied: unfolds with the explanation of the engine, in black and white.
    const denied = wrapper.get('[data-entry="4"]')
    expect(denied.attributes('data-error')).toBeUndefined()
    await denied.get('[data-action="toggle"]').trigger('click')
    expect(denied.get('[data-detail="policy"]').text()).toBe(
      'refusé — règle « destructive » de Kubernetes Admin en PROD, verrouillée',
    )
    // Leak masked: counted and named, never shown.
    const leak = wrapper.get('[data-entry="5"]')
    await leak.get('[data-action="toggle"]').trigger('click')
    expect(leak.get('[data-detail="masked"]').text()).toBe(
      '2 valeurs masquées (secret connu, en-tête Authorization)',
    )
    wrapper.unmount()
  })

  // Exigence : EX-018-03
  it('shows a request or a response as text, never as HTML', async () => {
    const [first] = exampleEntries().filter((e) => e.seq === 2)
    if (!first?.details) throw new Error('fixture')
    const crafted: JournalEntry = {
      ...first,
      client: '<b>client</b>',
      details: { ...first.details, response: { text: '<img src=x onerror="alert(1)">' } },
    }
    const { wrapper } = await mountTab([crafted])
    await wrapper.get('[data-action="toggle"]').trigger('click')
    expect(wrapper.find('img').exists()).toBe(false)
    expect(wrapper.find('[data-entry] b').exists()).toBe(false)
    expect(wrapper.get('[data-detail="response"]').text()).toContain('<img src=x onerror=\\"alert(1)\\">')
    expect(wrapper.get('[data-cell="client"]').text()).toBe('<b>client</b>')
    wrapper.unmount()
  })

  // Exigence : EX-018-02
  it('filters by status and by environment, and highlights the search', async () => {
    const { wrapper } = await mountTab()
    await wrapper.get('[data-status-filter="problems"]').trigger('click')
    expect(shownSeqs(wrapper)).toEqual([10, 7, 5, 3, 2])
    expect(wrapper.get('[data-field="journal-count"]').text()).toBe('5 sur 10 entrées')
    await wrapper.get('[data-status-filter="policies"]').trigger('click')
    expect(shownSeqs(wrapper)).toEqual([7, 6, 4])
    await wrapper.get('[data-status-filter="all"]').trigger('click')

    await wrapper.get('[data-field="journal-search"]').setValue('kibána')
    expect(shownSeqs(wrapper)).toEqual([5, 3, 1])
    const marks = wrapper.findAll('[data-entry="5"] [data-highlight]').map((m) => m.text())
    expect(marks).toEqual(['Kibana', 'kibana'])
    await wrapper.get('[data-status-filter="problems"]').trigger('click')
    expect(shownSeqs(wrapper)).toEqual([5, 3])

    await wrapper.get('[data-field="journal-search"]').setValue('jira prod')
    expect(shownSeqs(wrapper)).toEqual([])
    expect(wrapper.get('[data-empty="filtered"]').text()).toContain('Aucune entrée ne correspond à « jira prod ».')
    await wrapper.get('[data-action="clear-filters"]').trigger('click')
    expect(shownSeqs(wrapper)).toHaveLength(10)
    wrapper.unmount()
  })

  // Exigence : EX-018-01
  it('pauses: the rows stay, a counter tells what arrived; « Afficher » resumes', async () => {
    const { wrapper, emitJournal } = await mountTab()
    const live = wrapper.get('[data-action="live"]')
    expect(live.text()).toBe('En direct')
    await live.trigger('click')
    expect(live.text()).toBe('En pause')
    expect(wrapper.get('[data-field="pending"]').text()).toBe('Aucune nouvelle entrée depuis la pause.')
    const [last] = exampleEntries().slice(-1)
    if (!last) throw new Error('fixture')
    emitJournal({ type: 'entry', entry: { ...last, seq: 11, summary: 'panic à a.rs:1:1 : un' } })
    emitJournal({ type: 'entry', entry: { ...last, seq: 12, summary: 'panic à a.rs:1:1 : deux' } })
    await flushPromises()
    expect(shownSeqs(wrapper)[0]).toBe(10)
    expect(wrapper.get('[data-field="pending"]').attributes('data-pending')).toBe('2')
    expect(wrapper.get('[data-field="pending"]').text()).toContain('2 nouvelles entrées depuis la pause.')
    await wrapper.get('[data-action="show-pending"]').trigger('click')
    expect(shownSeqs(wrapper).slice(0, 2)).toEqual([12, 11])
    expect(wrapper.find('[data-field="pending"]').exists()).toBe(false)
    expect(live.text()).toBe('En direct')
    wrapper.unmount()
  })

  // Exigence : EX-018-04
  it('exports a diagnostic and says where, or why not; closing the dialog says nothing', async () => {
    const { wrapper, backend } = await mountTab()
    await wrapper.get('[data-action="export"]').trigger('click')
    await flushPromises()
    expect(backend.exportDiagnostic).toHaveBeenCalledTimes(1)
    expect(wrapper.get('[data-banner="export-saved"]').text()).toContain(
      'Diagnostic exporté : C:\\Users\\exemple\\diagnostic.zip',
    )
    await wrapper.get('[data-banner="export-saved"]').get('button').trigger('click')
    expect(wrapper.find('[data-banner="export-saved"]').exists()).toBe(false)

    backend.exportDiagnostic.mockResolvedValueOnce({ status: 'cancelled' })
    await wrapper.get('[data-action="export"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-banner^="export"]').exists()).toBe(false)

    backend.exportDiagnostic.mockRejectedValueOnce('export de diagnostic impossible : accès refusé')
    await wrapper.get('[data-action="export"]').trigger('click')
    await flushPromises()
    const failed = wrapper.get('[data-banner="export-failed"]')
    expect(failed.attributes('data-tone')).toBe('error')
    expect(failed.text()).toContain('Export de diagnostic impossible : accès refusé.')
    wrapper.unmount()
  })

  it('says so when the journal is empty', async () => {
    const { wrapper } = await mountTab([])
    expect(wrapper.get('[data-empty="journal"]').text()).toContain('Aucune entrée pour le moment.')
    expect(wrapper.find('[data-action="clear-filters"]').exists()).toBe(false)
    wrapper.unmount()
  })

  it('shows the information alerts meant for the diagnostic, which the MCP tab leaves out', async () => {
    const snapshot = snapshotOf('sessions')
    snapshot.alerts = [
      { level: 'info', kind: 'startupHardening', message: "Exclusion des rapports d'erreur Windows non posée." },
      { level: 'info', kind: 'catalogCache', message: 'rec/jira : cache ignoré.' },
    ]
    const { wrapper } = await mountTab([], snapshot)
    expect(wrapper.find('[data-banner="startupHardening"]').exists()).toBe(true)
    expect(wrapper.find('[data-banner="catalogCache"]').exists()).toBe(false)
    wrapper.unmount()

    const mcp = mount(inTooltips(McpTab, { snapshot }), {
      global: { provide: { [STORE as symbol]: createStore(spyBackend(snapshot).backend) } },
    })
    expect(mcp.find('[data-banner="catalogCache"]').exists()).toBe(true)
    expect(mcp.find('[data-banner="startupHardening"]').exists()).toBe(false)
    mcp.unmount()
  })
})
