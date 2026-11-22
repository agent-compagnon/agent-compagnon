// The application's backend: the commands of the window `main` (declared in `app/build.rs`, granted by
// `app/capabilities/main.json`) and the `snapshot-changed` event. A snapshot that is not exactly the expected shape
// is refused rather than half shown.

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { Backend } from './backend'
import { type AppSnapshot, parseSnapshot } from './types'

export const SNAPSHOT_CHANGED = 'snapshot-changed'

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
}

/** Whether the page runs in a window of the application (the IPC of Tauri is there). */
export function insideTauri(win: object = window): boolean {
  return '__TAURI_INTERNALS__' in win
}
