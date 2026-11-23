import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Backend } from '@/api/backend'
import type { DoctorReport } from '@/api/types'
import { STORE, createStore } from '@/store/snapshot'
import { doctorFixture, snapshotOf, spyBackend } from '@/test/fixtures'
import DoctorDialog from './DoctorDialog.vue'
import { shownFix } from './model'

function mountDialog(backend: Backend) {
  const store = createStore(backend)
  return mount(DoctorDialog, {
    props: { open: true },
    global: { provide: { [STORE as symbol]: store } },
    attachTo: document.body,
  })
}

const $ = (selector: string) => document.body.querySelector<HTMLElement>(selector)
/** The text shown, its no-break spaces read as spaces. */
const shown = (element: Element | null | undefined) => element?.textContent.replaceAll('\u00a0', ' ') ?? ''
const $$ = (selector: string) => [...document.body.querySelectorAll<HTMLElement>(selector)]
const check = (id: string) => $(`[data-check="${id}"]`)

async function click(element: HTMLElement | null | undefined) {
  if (!element) throw new Error('missing element')
  element.click()
  await flushPromises()
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('dialog « Auto-diagnostic »', () => {
  // Exigence : EX-024-12
  it('shows the run in progress, then every check of the report', async () => {
    let finish: (report: DoctorReport) => void = () => undefined
    const spy = spyBackend(snapshotOf('sessions'))
    spy.backend.runDoctor.mockImplementationOnce(
      () =>
        new Promise<DoctorReport>((resolve) => {
          finish = resolve
        }),
    )
    const wrapper = mountDialog(spy.backend)
    await flushPromises()
    expect($('[data-dialog="doctor"]')?.dataset.phase).toBe('running')
    expect($('[data-running]')?.textContent).toContain('Contrôle en cours…')
    expect($('[data-action="doctor-copy"]')?.hasAttribute('disabled')).toBe(true)
    finish(doctorFixture('ok'))
    await flushPromises()
    expect($('[data-dialog="doctor"]')?.dataset.phase).toBe('done')
    expect($$('[data-check]')).toHaveLength(11)
    expect($$('[data-check]').every((e) => e.dataset.status === 'ok')).toBe(true)
    expect($('[data-summary]')?.textContent.trim()).toBe('Tout va bien.')
    expect($('[data-file]')?.textContent).toContain('doctor-20261123-100000.txt')
    expect(spy.backend.runDoctor).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  // Exigence : EX-024-12
  it('says what to do, in orange for a warning and in red for a failure', async () => {
    const spy = spyBackend(snapshotOf('sessions'))
    spy.backend.runDoctor.mockResolvedValueOnce(doctorFixture('warnings'))
    const wrapper = mountDialog(spy.backend)
    await flushPromises()
    expect(shown($('[data-summary]'))).toContain('8 avertissements : une action est attendue.')
    expect($('[data-summary]')?.className).toContain('text-attention-text')
    const notifications = check('notifications')
    expect(notifications?.querySelector('[data-check-status]')?.textContent).toBe('Avertissement')
    expect(notifications?.querySelector('[data-check-status]')?.className).toContain('text-attention-text')
    expect(shown(notifications?.querySelector('[data-fix]'))).toContain('« Système », « Notifications »')
    // No line break inside « … » nor before « : » on the screen.
    expect(notifications?.querySelector('[data-fix]')?.textContent).toContain('«\u00a0Notifications\u00a0»\u00a0:')
    expect(check('config')?.querySelector('[data-check-status]')?.className).not.toMatch(/attention|destructive/)
    wrapper.unmount()

    spy.backend.runDoctor.mockResolvedValueOnce(doctorFixture('failures'))
    const failed = mountDialog(spy.backend)
    await flushPromises()
    expect($('[data-summary]')?.textContent).toContain('9 échecs')
    expect(check('webview')?.querySelector('[data-check-status]')?.className).toContain('text-destructive-text')
    expect(check('webview')?.querySelector('[data-fix]')?.textContent).toContain('WebView2 Runtime')
    // One advice for eleven servers that fail the same way: said once.
    const mcp = check('mcp')
    expect(mcp?.querySelectorAll('[data-finding="failure"]')).toHaveLength(11)
    expect(mcp?.querySelectorAll('[data-fix]')).toHaveLength(1)
    failed.unmount()
  })

  // Exigence : EX-024-12
  it('« Copier » copies the text of the report, or shows it when the clipboard refuses', async () => {
    const writeText = vi.fn(() => Promise.resolve())
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    const spy = spyBackend(snapshotOf('sessions'))
    const wrapper = mountDialog(spy.backend)
    await flushPromises()
    await click($('[data-action="doctor-copy"]'))
    expect(writeText).toHaveBeenCalledWith(doctorFixture('ok').text)
    expect($('[data-copy]')?.textContent).toContain('Rapport copié')
    expect($('[data-copied]')).toBeNull()

    writeText.mockImplementationOnce(() => Promise.reject(new Error('refusé')))
    await click($('[data-action="doctor-copy"]'))
    expect($('[data-copy]')?.textContent).toContain('Copie refusée')
    expect($('[data-copied]')?.textContent).toBe(doctorFixture('ok').text)

    // « Relancer » runs every check again.
    await click($('[data-action="doctor-again"]'))
    expect(spy.backend.runDoctor).toHaveBeenCalledTimes(2)
    expect($('[data-copy]')).toBeNull()
    wrapper.unmount()
  })

  // Exigence : EX-024-12
  it('shows why no report could be given', async () => {
    const spy = spyBackend(snapshotOf('sessions'))
    spy.backend.runDoctor.mockRejectedValueOnce('filtre de sortie indisponible')
    const wrapper = mountDialog(spy.backend)
    await flushPromises()
    expect(shown($('[data-error]'))).toContain('Auto-diagnostic impossible : filtre de sortie indisponible.')
    expect($('[data-check]')).toBeNull()
    wrapper.unmount()
  })

  it('says an advice once per check', () => {
    const mcp = doctorFixture('failures').checks.find((c) => c.id === 'mcp')
    if (!mcp) throw new Error('no MCP check')
    expect(mcp.findings.map((_, i) => shownFix(mcp, i) !== null)).toEqual(mcp.findings.map((_, i) => i === 0))
  })
})
