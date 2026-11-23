// What the dialog « Clients LLM » shows, as pure functions of the answers of the application (spec 023).

import type { ClientLocation, ClientPreview, ClientsView, EntrySummary, LocationId } from '@/api/types'
import { fr } from '@/i18n/fr'

export function labelOf(id: LocationId): string {
  return fr.clients.label[id]
}

/** « Installer » is offered for a client that is there, when MCP Proxy is installed and the entry is not ours yet. */
export function canInstall(view: ClientsView, location: ClientLocation): boolean {
  return view.command !== null && location.state.kind !== 'notInstalled' && location.state.kind !== 'configured'
}

/** « Retirer » for an entry found, or a file that only the user can clean (the preview says how). */
export function canRemove(location: ClientLocation): boolean {
  return ['configured', 'otherCommand', 'unreadable'].includes(location.state.kind)
}

/** Both files of Claude Desktop exist: the installation reads only one of them. */
export function twoClaudeLocations(view: ClientsView): boolean {
  const present = (id: LocationId) => view.locations.some((l) => l.id === id && l.state.kind !== 'notInstalled')
  return present('claude-desktop') && present('claude-desktop-msix')
}

/** The documented file, when the MSIX package is there: Claude Desktop most likely reads the other one. */
export function probablyIgnored(view: ClientsView, location: ClientLocation): boolean {
  return location.id === 'claude-desktop' && twoClaudeLocations(view)
}

/** What the row says under its state; empty when the state says it all. */
export function detail(location: ClientLocation): string {
  const state = location.state
  if (state.kind === 'otherCommand') return fr.clients.otherCommand(state.previous?.command ?? null)
  if (state.kind === 'unreadable') return fr.clients.unreadable(state.reason ?? '')
  return ''
}

/** `"mcp-proxy": { "command": "…", … }` for an entry found: its command, the names of its other fields only. */
function previousLine(previous: EntrySummary): string {
  const command = previous.command === null ? '…' : JSON.stringify(previous.command)
  const others = previous.otherKeys.length === 0 ? '' : `, … (${fr.clients.preview.otherFields(previous.otherKeys)})`
  return `"mcp-proxy": { "command": ${command}${others} }`
}

export interface DiffLine {
  sign: '-' | '+'
  text: string
}

/** The lines the change removes, then the lines it adds. */
export function diffLines(preview: ClientPreview): DiffLine[] {
  const lines: DiffLine[] = []
  if (preview.previous !== null && (preview.change === 'replace' || preview.change === 'remove')) {
    lines.push({ sign: '-', text: previousLine(preview.previous) })
  }
  if (preview.entry !== null) {
    for (const text of preview.entry.split('\n')) lines.push({ sign: '+', text })
  }
  return lines
}

/** The sentence that says what the preview changes. */
export function changeText(preview: ClientPreview): string {
  const texts = fr.clients.preview
  switch (preview.change) {
    case 'create':
      return texts.create
    case 'add':
      return texts.add
    case 'replace':
      return texts.replace
    case 'remove':
      return texts.remove
    case 'nothing':
      return texts.nothing[preview.action]
    case 'manual':
      return texts.manual(preview.reason ?? '')
  }
}

/** Whether the user can apply the preview: something to write, and it can be written safely. */
export function applicable(preview: ClientPreview): boolean {
  return preview.change !== 'nothing' && preview.change !== 'manual'
}
