import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import BasicPrompt from './BasicPrompt.vue'
import { type BasicAnswer, type BasicPromptData, promptData, sendAnswer } from './prompt'

const DATA: BasicPromptData = {
  title: 'Connexion — SSO Outils',
  login: null,
  error: null,
  attempt: 1,
  maxAttempts: 3,
  command: 'submit_basic_credentials',
}

function field(wrapper: ReturnType<typeof mount>, name: string): HTMLInputElement {
  return wrapper.find(`input[name="${name}"]`).element as HTMLInputElement
}

describe('Basic window', () => {
  // Exigence : EX-012-12
  it('sends the login and password, then empties the form', async () => {
    const send = vi.fn<(answer: BasicAnswer | null) => void>()
    const wrapper = mount(BasicPrompt, { props: { data: DATA, send } })
    expect(wrapper.find('h1').text()).toBe('Connexion — SSO Outils')
    await wrapper.find('input[name="login"]').setValue('alice')
    await wrapper.find('input[name="password"]').setValue('s3cret-pw')
    await wrapper.find('form').trigger('submit')
    expect(send).toHaveBeenCalledTimes(1)
    expect(send).toHaveBeenCalledWith({ login: 'alice', password: 's3cret-pw' })
    expect(field(wrapper, 'login').value).toBe('')
    expect(field(wrapper, 'password').value).toBe('')
    expect(wrapper.html()).not.toContain('s3cret-pw')
    // A single answer per window.
    await wrapper.find('form').trigger('submit')
    await wrapper.find('button[type="button"]').trigger('click')
    expect(send).toHaveBeenCalledTimes(1)
  })

  // Exigence : EX-012-12
  it('answers nothing but a cancellation on « Annuler »', async () => {
    const send = vi.fn<(answer: BasicAnswer | null) => void>()
    const wrapper = mount(BasicPrompt, { props: { data: DATA, send } })
    await wrapper.find('input[name="password"]').setValue('typed')
    const cancel = wrapper.find('button[type="button"]')
    expect(cancel.text()).toBe('Annuler')
    await cancel.trigger('click')
    expect(send).toHaveBeenCalledExactlyOnceWith(null)
    expect(field(wrapper, 'password').value).toBe('')
  })

  // Exigence : EX-012-12
  it('shows the refusal and fills in the previous login, never a password', () => {
    const data = { ...DATA, login: 'alice', error: 'identifiants refusés', attempt: 2 }
    const wrapper = mount(BasicPrompt, { props: { data, send: vi.fn() } })
    expect(wrapper.find('[role="alert"]').text()).toBe('identifiants refusés')
    expect(wrapper.find('.attempt').text()).toBe('Essai 2 sur 3')
    expect(field(wrapper, 'login').value).toBe('alice')
    expect(field(wrapper, 'password').value).toBe('')
    expect(field(wrapper, 'password').type).toBe('password')
  })

  // Exigence : EX-012-12
  it('reads its data from the application and sends through its command only', async () => {
    expect(promptData({})).toBeNull()
    expect(promptData({ __MCP_PROXY_BASIC__: { title: 1 } })).toBeNull()
    expect(promptData({ __MCP_PROXY_BASIC__: DATA })).toEqual(DATA)

    const invoke = vi.fn<(cmd: string, args: Record<string, unknown>) => Promise<unknown>>(() =>
      Promise.reject(new Error('refused')),
    )
    sendAnswer({ __TAURI_INTERNALS__: { invoke } }, DATA.command, { login: 'alice', password: 'pw' })
    expect(invoke).toHaveBeenCalledExactlyOnceWith('submit_basic_credentials', {
      answer: { login: 'alice', password: 'pw' },
    })
    sendAnswer({ __TAURI_INTERNALS__: { invoke } }, DATA.command, null)
    expect(invoke).toHaveBeenLastCalledWith('submit_basic_credentials', { answer: null })
    expect(() => {
      sendAnswer({}, DATA.command, null)
    }).not.toThrow()
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
})
