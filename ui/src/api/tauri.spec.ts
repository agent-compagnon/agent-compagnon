import { beforeEach, describe, expect, it, vi } from 'vitest'
import buildRs from '../../../app/build.rs?raw'
import mainCapability from '../../../app/capabilities/main.json'
import example from './snapshot.example.json'

const invoke = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>()
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => undefined)) }))

const { InvalidSnapshot, insideTauri, tauriBackend } = await import('./tauri')

describe("the application's backend", () => {
  beforeEach(() => {
    invoke.mockReset()
    invoke.mockResolvedValue(null)
  })

  // Exigence : EX-016-07
  // Exigence : EX-017-04
  it('sends each command of the window with identifiers only', async () => {
    await tauriBackend.connectSession('sso-dev')
    await tauriBackend.disconnectSession('sso-dev')
    await tauriBackend.setProfile('support')
    await tauriBackend.resumeGateway()
    await tauriBackend.showConfirmations()
    await tauriBackend.setEnvironmentExposed('prod', true)
    await tauriBackend.setMcpExposed('kibana-logs', false)
    await tauriBackend.acknowledgeMcpUpdate('dev', 'gitlab-ops')
    await tauriBackend.reloadMcp('dev', 'gitlab-ops')
    expect(invoke.mock.calls).toEqual([
      ['connect_session', { session: 'sso-dev' }],
      ['disconnect_session', { session: 'sso-dev' }],
      ['set_profile', { profile: 'support' }],
      ['resume_gateway'],
      ['show_confirmations'],
      ['set_environment_exposed', { environment: 'prod', exposed: true }],
      ['set_mcp_exposed', { mcp: 'kibana-logs', exposed: false }],
      ['acknowledge_mcp_update', { environment: 'dev', mcp: 'gitlab-ops' }],
      ['reload_mcp', { environment: 'dev', mcp: 'gitlab-ops' }],
    ])
    // Every command is declared by the application and granted to the main window, and to it only.
    for (const [command] of invoke.mock.calls) {
      expect(buildRs).toContain(`"${command}"`)
      expect(mainCapability.permissions).toContain(`allow-${command.replaceAll('_', '-')}`)
    }
    expect(mainCapability.windows).toEqual(['main'])
  })

  // Exigence : EX-016-06
  it('refuses a snapshot that is not exactly the expected one', async () => {
    invoke.mockResolvedValueOnce(structuredClone(example))
    expect((await tauriBackend.getSnapshot()).version).toBe('0.1.0')
    invoke.mockResolvedValueOnce({ ...structuredClone(example), secret: 'x' })
    await expect(tauriBackend.getSnapshot()).rejects.toBeInstanceOf(InvalidSnapshot)
    expect(insideTauri({})).toBe(false)
    expect(insideTauri({ __TAURI_INTERNALS__: {} })).toBe(true)
  })
})
