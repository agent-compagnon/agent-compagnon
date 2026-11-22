import { beforeEach, describe, expect, it, vi } from 'vitest'
import buildRs from '../../../app/build.rs?raw'
import mainCapability from '../../../app/capabilities/main.json'
import journalExample from './journal.example.json'
import example from './snapshot.example.json'

const invoke = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>()
type Handler = (event: { payload: unknown }) => void
const handlers = new Map<string, Handler>()
const listen = vi.fn((name: string, handler: Handler) => {
  handlers.set(name, handler)
  return Promise.resolve(() => handlers.delete(name))
})
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen }))

const { InvalidSnapshot, insideTauri, tauriBackend } = await import('./tauri')
const { InvalidJournal } = await import('./backend')

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
    invoke.mockResolvedValueOnce(structuredClone(journalExample.getJournal))
    await tauriBackend.getJournal()
    invoke.mockResolvedValueOnce({ status: 'cancelled' })
    await tauriBackend.exportDiagnostic()
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
      ['get_journal'],
      ['export_diagnostic'],
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

  // Exigence : EX-018-01
  // Exigence : EX-018-12
  it('follows the journal: each entry, a resynchronisation, and an unreadable entry asks again', async () => {
    const events: unknown[] = []
    const stop = await tauriBackend.onJournal((event) => events.push(event))
    expect([...handlers.keys()].sort()).toEqual(['journal-entry', 'journal-resync'])
    const [entry] = structuredClone(journalExample.getJournal.entries)
    handlers.get('journal-entry')?.({ payload: entry })
    handlers.get('journal-resync')?.({ payload: null })
    handlers.get('journal-entry')?.({ payload: { ...entry, extra: 1 } })
    expect(events).toEqual([{ type: 'entry', entry }, { type: 'resync' }, { type: 'resync' }])
    stop()
    expect(handlers.size).toBe(0)
  })

  // Exigence : EX-018-04
  // Exigence : EX-018-10
  it('refuses a journal or an export answer that is not exactly the expected one', async () => {
    invoke.mockResolvedValueOnce({ entries: [], secret: 'x' })
    await expect(tauriBackend.getJournal()).rejects.toBeInstanceOf(InvalidJournal)
    invoke.mockResolvedValueOnce({ status: 'saved', path: 'C:\\diag.zip' })
    await expect(tauriBackend.exportDiagnostic()).resolves.toEqual({ status: 'saved', path: 'C:\\diag.zip' })
    invoke.mockResolvedValueOnce({ status: 'done' })
    await expect(tauriBackend.exportDiagnostic()).rejects.toThrow('réponse illisible')
    invoke.mockRejectedValueOnce('export de diagnostic impossible : accès refusé')
    await expect(tauriBackend.exportDiagnostic()).rejects.toBe('export de diagnostic impossible : accès refusé')
  })
})
