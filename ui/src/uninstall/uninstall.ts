// What the uninstallation window of spec 022 receives and sends. The data is defined by the application before the
// page runs (`window.__MCP_PROXY_UNINSTALL__`, frozen); every answer goes to the window's only command, which
// decides. « Désinstaller » resolves with the report, or rejects with why nothing was changed (a French text written
// by the application). Both forms are fixed by `uninstall.example.json`, written by a test of the `deploy` crate.

export type ClientEntry = 'ours' | 'other' | 'unreadable'

export interface UninstallClient {
  location: string
  label: string
  file: string
  entry: ClientEntry
  detail: string | null
}

export interface UninstallData {
  version: string
  location: string
  data: string
  clients: UninstallClient[]
  command: string
}

export interface LeftItem {
  what: string
  why: string
  failed: boolean
}

export interface UninstallReport {
  removed: string[]
  atClose: string[]
  left: LeftItem[]
  keptInstalled: boolean
}

export type UninstallAnswer =
  | { choice: 'uninstall'; removeClients: boolean }
  | { choice: 'cancel' }
  | { choice: 'close' }

/** Sends an answer: « Désinstaller » resolves with the report; the others once the application acted. */
export type Send = (answer: UninstallAnswer) => Promise<UninstallReport | null>

type Invoke = (cmd: string, args: Record<string, unknown>) => Promise<unknown>

export interface UninstallWindow {
  __MCP_PROXY_UNINSTALL__?: unknown
  __TAURI_INTERNALS__?: { invoke?: Invoke }
}

type Fields = Record<string, unknown>

function object(value: unknown): value is Fields {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Exactly these keys, no other. */
function keys(value: Fields, expected: readonly string[]): boolean {
  const found = Object.keys(value)
  return found.length === expected.length && expected.every((key) => key in value)
}

function strings(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === 'string')
}

function isClient(value: unknown): value is UninstallClient {
  if (!object(value) || !keys(value, ['location', 'label', 'file', 'entry', 'detail'])) return false
  return (
    typeof value.location === 'string' &&
    typeof value.label === 'string' &&
    typeof value.file === 'string' &&
    (value.entry === 'ours' || value.entry === 'other' || value.entry === 'unreadable') &&
    (value.detail === null || typeof value.detail === 'string')
  )
}

export function isData(value: unknown): value is UninstallData {
  if (!object(value) || !keys(value, ['version', 'location', 'data', 'clients', 'command'])) return false
  return (
    typeof value.version === 'string' &&
    typeof value.location === 'string' &&
    typeof value.data === 'string' &&
    Array.isArray(value.clients) &&
    value.clients.every(isClient) &&
    typeof value.command === 'string'
  )
}

function isLeft(value: unknown): value is LeftItem {
  return (
    object(value) &&
    keys(value, ['what', 'why', 'failed']) &&
    typeof value.what === 'string' &&
    typeof value.why === 'string' &&
    typeof value.failed === 'boolean'
  )
}

export function isReport(value: unknown): value is UninstallReport {
  if (!object(value) || !keys(value, ['removed', 'atClose', 'left', 'keptInstalled'])) return false
  return (
    strings(value.removed) &&
    strings(value.atClose) &&
    Array.isArray(value.left) &&
    value.left.every(isLeft) &&
    typeof value.keptInstalled === 'boolean'
  )
}

export function uninstallData(win: UninstallWindow): UninstallData | null {
  const data = win.__MCP_PROXY_UNINSTALL__
  return isData(data) ? data : null
}

/** The French reason of a refused answer, as the application words it; a fixed text otherwise. */
export function failure(error: unknown): string {
  if (typeof error === 'string' && error.length > 0) return error
  if (error instanceof Error && error.message.length > 0) return error.message
  return "la désinstallation n'a pas répondu"
}

/** Sends through the window's command; without the IPC (outside the application), nothing can be done. */
export function sender(win: UninstallWindow, command: string): Send {
  return async (answer) => {
    const invoke = win.__TAURI_INTERNALS__?.invoke
    if (typeof invoke !== 'function') throw new Error('MCP Proxy ne répond pas')
    const reply = await invoke(command, { answer })
    if (answer.choice !== 'uninstall') return null
    if (!isReport(reply)) throw new Error('compte rendu illisible')
    return reply
  }
}
