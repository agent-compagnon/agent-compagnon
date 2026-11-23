// The application's backend: the commands of the window `main` (declared in `app/build.rs`, granted by
// `app/capabilities/main.json`) and the `snapshot-changed`, `journal-entry` and `journal-resync` events. A snapshot,
// a journal or an entry that is not exactly the expected shape is refused rather than half shown.

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { type Backend, InvalidJournal } from './backend'
import {
  type AppSnapshot,
  parseApplyOutcome,
  parseClientPreview,
  parseClients,
  parseDoctorReport,
  type ExportOutcome,
  type JournalEntry,
  parseExportOutcome,
  parseEntry,
  parseJournal,
  parseSnapshot,
} from './types'

export const SNAPSHOT_CHANGED = 'snapshot-changed'
export const JOURNAL_ENTRY = 'journal-entry'
export const JOURNAL_RESYNC = 'journal-resync'

export class InvalidSnapshot extends Error {
  constructor() {
    super('état illisible')
  }
}


function checked(value: unknown): AppSnapshot {
  const snapshot = parseSnapshot(value)
  if (snapshot === null) throw new InvalidSnapshot()
  return snapshot
}

/** An answer of a dialog (« Clients LLM », « Auto-diagnostic ») that is not exactly the expected one: nothing of it
 * is shown. */
function read<T>(value: T | null): T {
  if (value === null) throw new Error('réponse illisible')
  return value
}

export const tauriBackend: Backend = {
  async getSnapshot() {
    return checked(await invoke('get_snapshot'))
  },
  async onSnapshot(listener) {
    return listen<unknown>(SNAPSHOT_CHANGED, (event) => {
      listener(parseSnapshot(event.payload))
    })
  },
  async connectSession(session) {
    await invoke('connect_session', { session })
  },
  async disconnectSession(session) {
    await invoke('disconnect_session', { session })
  },
  async setProfile(profile) {
    await invoke('set_profile', { profile })
  },
  async resumeGateway() {
    await invoke('resume_gateway')
  },
  async showConfirmations() {
    await invoke('show_confirmations')
  },
  async setEnvironmentExposed(environment, exposed) {
    await invoke('set_environment_exposed', { environment, exposed })
  },
  async setMcpExposed(mcp, exposed) {
    await invoke('set_mcp_exposed', { mcp, exposed })
  },
  async acknowledgeMcpUpdate(environment, mcp) {
    await invoke('acknowledge_mcp_update', { environment, mcp })
  },
  async reloadMcp(environment, mcp) {
    await invoke('reload_mcp', { environment, mcp })
  },
  async getJournal(): Promise<JournalEntry[]> {
    const entries = parseJournal(await invoke('get_journal'))
    if (entries === null) throw new InvalidJournal()
    return entries
  },
  async onJournal(listener) {
    const stopEntries = await listen<unknown>(JOURNAL_ENTRY, (event) => {
      const entry = parseEntry(event.payload)
      // An entry that cannot be read is asked again with the whole journal, which then says it is unreadable.
      listener(entry === null ? { type: 'resync' } : { type: 'entry', entry })
    })
    const stopResync = await listen<unknown>(JOURNAL_RESYNC, () => {
      listener({ type: 'resync' })
    })
    return () => {
      stopEntries()
      stopResync()
    }
  },
  async restartForUpdate() {
    await invoke('restart_for_update')
  },
  async exportDiagnostic(): Promise<ExportOutcome> {
    const outcome = parseExportOutcome(await invoke('export_diagnostic'))
    if (outcome === null) throw new Error('réponse illisible')
    return outcome
  },
  async getLlmClients() {
    return read(parseClients(await invoke('get_llm_clients')))
  },
  async previewLlmClient(location, action) {
    return read(parseClientPreview(await invoke('preview_llm_client', { location, action })))
  },
  async applyLlmClient(location, action, fingerprint) {
    return read(parseApplyOutcome(await invoke('apply_llm_client', { location, action, fingerprint })))
  },
  async runDoctor() {
    return read(parseDoctorReport(await invoke('run_doctor')))
  },
}

/** Whether the page runs in a window of the application (the IPC of Tauri is there). */
export function insideTauri(win: object = window): boolean {
  return '__TAURI_INTERNALS__' in win
}
