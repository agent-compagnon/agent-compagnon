// What the installation window of spec 019 receives and sends. The data is defined by the application before the
// page runs (`window.__MCP_PROXY_INSTALL__`, frozen); every answer goes to the window's only command, which
// decides. A failed installation comes back as the command's error, a French text written by the application.

export interface InstallData {
  kind: 'install' | 'update'
  version: string
  installedVersion: string | null
  autostart: boolean
  location: string
  command: string
}

export type InstallAnswer =
  | { choice: 'install'; autostart: boolean }
  | { choice: 'update' }
  | { choice: 'later' }
  | { choice: 'continue' }
  | { choice: 'quit' }

/** Sends an answer: resolves once the application acted (the window then closes), rejects with the reason. */
export type Send = (answer: InstallAnswer) => Promise<void>

type Invoke = (cmd: string, args: Record<string, unknown>) => Promise<unknown>

export interface InstallWindow {
  __MCP_PROXY_INSTALL__?: unknown
  __TAURI_INTERNALS__?: { invoke?: Invoke }
}

function isData(value: unknown): value is InstallData {
  if (typeof value !== 'object' || value === null) return false
  const data = value as Record<string, unknown>
  const installed = data.installedVersion
  return (
    (data.kind === 'install' || data.kind === 'update') &&
    typeof data.version === 'string' &&
    (installed === null || typeof installed === 'string') &&
    (data.kind === 'install' ? installed === null : typeof installed === 'string') &&
    typeof data.autostart === 'boolean' &&
    typeof data.location === 'string' &&
    typeof data.command === 'string'
  )
}

export function installData(win: InstallWindow): InstallData | null {
  const data = win.__MCP_PROXY_INSTALL__
  return isData(data) ? data : null
}

/** The French reason of a refused answer, as the application words it; a fixed text otherwise. */
export function failure(error: unknown): string {
  if (typeof error === 'string' && error.length > 0) return error
  if (error instanceof Error && error.message.length > 0) return error.message
  return "l'installation n'a pas répondu"
}

/** Sends through the window's command; without the IPC (outside the application), nothing can be done. */
export function sender(win: InstallWindow, command: string): Send {
  return async (answer) => {
    const invoke = win.__TAURI_INTERNALS__?.invoke
    if (typeof invoke !== 'function') throw new Error('MCP Proxy ne répond pas')
    await invoke(command, { answer })
  }
}
