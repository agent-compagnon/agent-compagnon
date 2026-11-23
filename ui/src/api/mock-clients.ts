// The dialog « Clients LLM » in the simulated backend (spec 023): each configuration file is a state, changed as
// `mcp_proxy_deploy::clients` would change it, with the same answers (preview, fingerprint, outcome) and the same
// journal lines. Built only in `--mode mock`, with the rest of the simulated backend.

import example from './clients.example.json'
import type {
  ApplyOutcome,
  ClientAction,
  ClientLocation,
  ClientPreview,
  ClientsView,
  EntrySummary,
  LocationId,
} from './types'

export const CLIENT_SCENARIOS = [
  'clients-etats',
  'clients-deux-emplacements',
  'clients-illisible',
  'clients-non-installe',
  'clients-modifie',
] as const

export type ClientScenario = (typeof CLIENT_SCENARIOS)[number]

const EXE = example.getLlmClients.command
const DOWNLOADED = 'C:\\Users\\exemple\\Downloads\\mcp-proxy.exe'

const LABELS: Record<LocationId, string> = {
  'claude-desktop': 'Claude Desktop',
  'claude-desktop-msix': 'Claude Desktop (paquet MSIX)',
  vscode: 'VS Code',
}

const FILES: Record<LocationId, { file: string; name: string; client: ClientLocation['client'] }> = {
  'claude-desktop': {
    file: '%APPDATA%\\Claude\\claude_desktop_config.json',
    name: 'claude_desktop_config.json',
    client: 'claudeDesktop',
  },
  'claude-desktop-msix': {
    file: '%LOCALAPPDATA%\\Packages\\Claude_pzs8sxrjxfjjc\\LocalCache\\Roaming\\Claude\\claude_desktop_config.json',
    name: 'claude_desktop_config.json',
    client: 'claudeDesktop',
  },
  vscode: { file: '%APPDATA%\\Code\\User\\mcp.json', name: 'mcp.json', client: 'vscode' },
}

type Kind = ClientLocation['state']['kind']

interface SimFile {
  kind: Kind
  fileExists: boolean
  previous: EntrySummary | null
  reason: string | null
  otherServers: number
  /** Bumped by every write: part of the fingerprint. */
  version: number
}

function file(kind: Kind, otherServers = 0, extra: Partial<SimFile> = {}): SimFile {
  const exists = kind !== 'notInstalled' && kind !== 'notConfigured'
  return { kind, fileExists: exists, previous: null, reason: null, otherServers, version: 0, ...extra }
}

const OLD_ENTRY: EntrySummary = { command: DOWNLOADED, otherKeys: ['args'] }

function filesOf(name: ClientScenario): Record<LocationId, SimFile> {
  switch (name) {
    case 'clients-deux-emplacements':
      return {
        'claude-desktop': file('notConfigured', 2, { fileExists: true }),
        'claude-desktop-msix': file('configured', 1),
        vscode: file('configured', 3),
      }
    case 'clients-illisible':
      return {
        'claude-desktop': file('unreadable', 0, { reason: 'JSON invalide à la ligne 12' }),
        'claude-desktop-msix': file('notInstalled'),
        vscode: file('notInstalled'),
      }
    case 'clients-non-installe':
      return {
        'claude-desktop': file('otherCommand', 1, { previous: OLD_ENTRY }),
        'claude-desktop-msix': file('notInstalled'),
        vscode: file('notConfigured', 0, { fileExists: true }),
      }
    case 'clients-etats':
    case 'clients-modifie':
      return {
        'claude-desktop': file('otherCommand', 2, { previous: OLD_ENTRY }),
        'claude-desktop-msix': file('notInstalled'),
        vscode: file('notConfigured', 1, { fileExists: true }),
      }
  }
}

function entry(location: LocationId): string {
  const command = JSON.stringify(EXE)
  const type = FILES[location].client === 'vscode' ? '  "type": "stdio",\n' : ''
  return `"mcp-proxy": {\n${type}  "command": ${command},\n  "args": ["stdio"]\n}`
}

function snippet(location: LocationId): string {
  const section = FILES[location].client === 'vscode' ? 'servers' : 'mcpServers'
  const body = entry(location).replaceAll('\n', '\n    ')
  return `{\n  "${section}": {\n    ${body}\n  }\n}\n`
}

function ours(location: LocationId): EntrySummary {
  const keys = FILES[location].client === 'vscode' ? ['type', 'args'] : ['args']
  return { command: EXE, otherKeys: keys }
}

/** `20261122-140307`, UTC. */
function stamp(at: number): string {
  const iso = new Date(at).toISOString()
  return `${iso.slice(0, 10).replaceAll('-', '')}-${iso.slice(11, 19).replaceAll(':', '')}`
}

export function createMockClients(
  name: ClientScenario,
  clock: () => number,
  recordEvent: (summary: string) => void,
) {
  const files = filesOf(name)
  const installed = name !== 'clients-non-installe'
  /** `clients-modifie`: the client rewrites its file once, between the first preview and its application. */
  let rewriteOnce = name === 'clients-modifie'

  const fingerprint = (location: LocationId, action: ClientAction) => {
    const index = Object.keys(files).indexOf(location)
    const text = `${String(index)}${action === 'install' ? '1' : '2'}${String(files[location].version)}`
    return text.padStart(64, '0')
  }

  function view(): ClientsView {
    return {
      command: installed ? EXE : null,
      snippet: installed ? example.getLlmClients.snippet : null,
      locations: (Object.keys(files) as LocationId[]).map((id) => {
        const f = files[id]
        return {
          id,
          client: FILES[id].client,
          file: FILES[id].file,
          state: { kind: f.kind, fileExists: f.fileExists, previous: f.previous, reason: f.reason },
          otherServers: f.otherServers,
        }
      }),
    }
  }

  function preview(location: LocationId, action: ClientAction): ClientPreview {
    const f = files[location]
    if (action === 'install' && !installed) {
      throw new Error(
        `${LABELS[location]} : MCP Proxy n'est pas installé : les clients LLM sont configurés avec l'exe installé, qui n'existe pas encore`,
      )
    }
    if (action === 'install' && f.kind === 'notInstalled') {
      throw new Error(`${LABELS[location]} n'est pas installé sur ce poste : rien n'est créé`)
    }
    const base: ClientPreview = {
      location,
      action,
      file: FILES[location].file,
      change: 'nothing',
      entry: null,
      previous: null,
      reason: null,
      snippet: null,
      backup: false,
      otherServers: f.otherServers,
      fingerprint: fingerprint(location, action),
    }
    if (f.kind === 'unreadable') {
      return { ...base, change: 'manual', reason: f.reason, snippet: action === 'install' ? snippet(location) : null }
    }
    if (action === 'install') {
      if (f.kind === 'configured') return base
      const change = f.kind === 'otherCommand' ? 'replace' : f.fileExists ? 'add' : 'create'
      return { ...base, change, entry: entry(location), previous: f.previous, backup: f.fileExists }
    }
    if (f.kind === 'configured' || f.kind === 'otherCommand') {
      return { ...base, change: 'remove', previous: f.previous ?? ours(location), backup: true }
    }
    return base
  }

  function apply(location: LocationId, action: ClientAction, sent: string): ApplyOutcome {
    const shown = preview(location, action)
    const f = files[location]
    if (rewriteOnce) {
      rewriteOnce = false
      f.version += 1
    }
    const label = LABELS[location]
    if (sent !== fingerprint(location, action)) {
      recordEvent(`configuration de ${label} changée depuis l'aperçu : rien n'a été écrit`)
      return { status: 'changed' }
    }
    if (shown.change === 'nothing' || shown.change === 'manual') return { status: 'nothing' }
    const backup = f.fileExists ? `${FILES[location].name}.bak-${stamp(clock())}` : null
    f.version += 1
    f.fileExists = true
    f.reason = null
    if (action === 'install') {
      f.kind = 'configured'
      f.previous = null
      recordEvent(`MCP Proxy ajouté à la configuration de ${label} par l'utilisateur`)
    } else {
      f.kind = 'notConfigured'
      f.previous = null
      recordEvent(`MCP Proxy retiré de la configuration de ${label} par l'utilisateur`)
    }
    return { status: 'applied', backup }
  }

  return {
    getLlmClients: () => Promise.resolve(view()),
    previewLlmClient: (location: LocationId, action: ClientAction) =>
      new Promise<ClientPreview>((resolve) => {
        resolve(preview(location, action))
      }),
    applyLlmClient: (location: LocationId, action: ClientAction, sent: string) =>
      new Promise<ApplyOutcome>((resolve) => {
        resolve(apply(location, action, sent))
      }),
  }
}
