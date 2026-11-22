// What the detail window of a confirmation (spec 013) receives and sends. The data is defined by the application
// before the page runs (`window.__MCP_PROXY_CONFIRM__`, frozen) and is already filtered; the answer goes to the
// window's only command. Closing the window refuses.

export interface ConfirmData {
  path: string
  client: string
  explanation: string
  arguments: string
  remainingMs: number
  command: string
}

export type ConfirmAnswer = 'approve' | 'refuse'

type Invoke = (cmd: string, args: Record<string, unknown>) => Promise<unknown>

export interface ConfirmWindow {
  __MCP_PROXY_CONFIRM__?: unknown
  __TAURI_INTERNALS__?: { invoke?: Invoke }
}

function isData(value: unknown): value is ConfirmData {
  if (typeof value !== 'object' || value === null) return false
  const data = value as Record<string, unknown>
  return (
    typeof data.path === 'string' &&
    typeof data.client === 'string' &&
    typeof data.explanation === 'string' &&
    typeof data.arguments === 'string' &&
    typeof data.remainingMs === 'number' &&
    Number.isFinite(data.remainingMs) &&
    typeof data.command === 'string'
  )
}

export function confirmData(win: ConfirmWindow): ConfirmData | null {
  const data = win.__MCP_PROXY_CONFIRM__
  return isData(data) ? data : null
}

/** Sends the answer and expects nothing back; an error of the IPC is swallowed. */
export function sendAnswer(win: ConfirmWindow, command: string, answer: ConfirmAnswer): void {
  const invoke = win.__TAURI_INTERNALS__?.invoke
  if (typeof invoke !== 'function') return
  invoke(command, { answer }).catch(() => undefined)
}

/** `0:30` for 30 000 ms, rounded up to the second; never negative. */
export function countdown(ms: number): string {
  const seconds = Math.max(0, Math.ceil(ms / 1000))
  const minutes = Math.floor(seconds / 60)
  return `${String(minutes)}:${String(seconds % 60).padStart(2, '0')}`
}
