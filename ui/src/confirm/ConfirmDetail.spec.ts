import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import ConfirmDetail from './ConfirmDetail.vue'
import { type ConfirmAnswer, type ConfirmData, confirmData, countdown, sendAnswer } from './detail'

const ARGUMENTS = JSON.stringify({ index: 'logs-2026.11', query: 'x'.repeat(500), lines: 'un\ndeux' }, null, 2)

const DATA: ConfirmData = {
  path: 'PROD / kibana-logs / delete_index',
  client: 'Claude Desktop',
  explanation: 'outil destructif en PROD : confirmation demandée',
  arguments: ARGUMENTS,
  remainingMs: 30_000,
  command: 'answer_confirmation',
}

describe('detail window of a confirmation', () => {
  let now = 0
  const clock = () => now

  beforeEach(() => {
    now = 0
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  // Exigence : EX-013-14
  it('shows the path, the client, the policy and every argument', () => {
    const wrapper = mount(ConfirmDetail, { props: { data: DATA, send: vi.fn(), now: clock } })
    expect(wrapper.find('.path').text()).toBe('PROD / kibana-logs / delete_index')
    expect(wrapper.find('.client').text()).toBe('Claude Desktop')
    expect(wrapper.find('.explanation').text()).toBe('outil destructif en PROD : confirmation demandée')
    expect(wrapper.find('pre.arguments').element.textContent).toBe(ARGUMENTS)
    expect(wrapper.find('[role="timer"]').text()).toContain('0:30')
    const labels = wrapper.findAll('button').map((b) => b.text())
    expect(labels).toEqual(['Refuser', 'Autoriser une fois'])
  })

  // Exigence : EX-013-14
  it('sends one answer only', async () => {
    const send = vi.fn<(answer: ConfirmAnswer) => void>()
    const wrapper = mount(ConfirmDetail, { props: { data: DATA, send, now: clock } })
    await wrapper.find('button.approve').trigger('click')
    await wrapper.find('button.approve').trigger('click')
    await wrapper.find('button.refuse').trigger('click')
    expect(send).toHaveBeenCalledExactlyOnceWith('approve')

    const refused = vi.fn<(answer: ConfirmAnswer) => void>()
    const other = mount(ConfirmDetail, { props: { data: DATA, send: refused, now: clock } })
    await other.find('button.refuse').trigger('click')
    await other.find('button.approve').trigger('click')
    expect(refused).toHaveBeenCalledExactlyOnceWith('refuse')
  })

  // Exigence : EX-013-14
  it('counts down and no longer approves once the delay is over', async () => {
    const send = vi.fn<(answer: ConfirmAnswer) => void>()
    const wrapper = mount(ConfirmDetail, { props: { data: DATA, send, now: clock } })
    now = 12_400
    await vi.advanceTimersByTimeAsync(250)
    expect(wrapper.find('[role="timer"]').text()).toContain('0:18')
    now = 30_000
    await vi.advanceTimersByTimeAsync(250)
    expect(wrapper.find('[role="timer"]').text()).toBe("Délai dépassé : l'appel est refusé.")
    expect(wrapper.find('button.approve').attributes('disabled')).toBeDefined()
    await wrapper.find('button.approve').trigger('click')
    expect(send).not.toHaveBeenCalled()
    // Late at the very click, before the timer saw it.
    const late = mount(ConfirmDetail, { props: { data: { ...DATA, remainingMs: 1_000 }, send, now: clock } })
    now = 31_500
    await late.find('button.approve').trigger('click')
    expect(send).not.toHaveBeenCalled()
    expect(countdown(-5)).toBe('0:00')
    expect(countdown(61_001)).toBe('1:02')
  })

  // Exigence : EX-013-14
  it('reads its data from the application and sends through its command only', async () => {
    expect(confirmData({})).toBeNull()
    expect(confirmData({ __MCP_PROXY_CONFIRM__: { ...DATA, remainingMs: 'x' } })).toBeNull()
    expect(confirmData({ __MCP_PROXY_CONFIRM__: { ...DATA, remainingMs: Number.NaN } })).toBeNull()
    expect(confirmData({ __MCP_PROXY_CONFIRM__: DATA })).toEqual(DATA)

    const invoke = vi.fn<(cmd: string, args: Record<string, unknown>) => Promise<unknown>>(() =>
      Promise.reject(new Error('refused')),
    )
    sendAnswer({ __TAURI_INTERNALS__: { invoke } }, DATA.command, 'approve')
    expect(invoke).toHaveBeenCalledExactlyOnceWith('answer_confirmation', { answer: 'approve' })
    expect(() => {
      sendAnswer({}, DATA.command, 'refuse')
    }).not.toThrow()
    await vi.advanceTimersByTimeAsync(0)
  })
})
