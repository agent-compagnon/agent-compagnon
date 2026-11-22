// What the Basic window of spec 012 receives and sends. The data is defined by the application before the page
// runs (`window.__MCP_PROXY_BASIC__`, frozen); the answer goes to the window's only command.

export interface BasicPromptData {
  title: string
  login: string | null
  error: string | null
  attempt: number
  maxAttempts: number
  command: string
}

export interface BasicAnswer {
  login: string
  password: string
}

type Invoke = (cmd: string, args: Record<string, unknown>) => Promise<unknown>

export interface BasicWindow {
  __MCP_PROXY_BASIC__?: unknown
  __TAURI_INTERNALS__?: { invoke?: Invoke }
}

function isData(value: unknown): value is BasicPromptData {
  if (typeof value !== 'object' || value === null) return false
  const data = value as Record<string, unknown>
  const optionalText = (v: unknown) => v === null || typeof v === 'string'
  return (
    typeof data.title === 'string' &&
    optionalText(data.login) &&
    optionalText(data.error) &&
    typeof data.attempt === 'number' &&
    typeof data.maxAttempts === 'number' &&
    typeof data.command === 'string'
  )
}

export function promptData(win: BasicWindow): BasicPromptData | null {
  const data = win.__MCP_PROXY_BASIC__
  return isData(data) ? data : null
}

/** Sends the answer (`null`: « Annuler ») and expects nothing back; an error of the IPC is swallowed. */
export function sendAnswer(win: BasicWindow, command: string, answer: BasicAnswer | null): void {
  const invoke = win.__TAURI_INTERNALS__?.invoke
  if (typeof invoke !== 'function') return
  invoke(command, { answer }).catch(() => undefined)
}
