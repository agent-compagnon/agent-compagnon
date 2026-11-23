// What the main window asks of the application. Two implementations: `tauri.ts` (the application, through the
// commands granted to the window `main`) and `mock.ts` (a simulated backend, built only in `--mode mock`, to run
// the window in a plain browser). The window never holds anything secret: it shows the snapshot and sends
// identifiers.

import type {
  AppSnapshot,
  ApplyOutcome,
  ClientAction,
  ClientPreview,
  ClientsView,
  ExportOutcome,
  JournalEntry,
  LocationId,
} from './types'

/** What follows `get_journal`: one new entry, or a sign that entries were lost on the way (ask again). */
export type JournalEvent = { type: 'entry'; entry: JournalEntry } | { type: 'resync' }

export interface Backend {
  /** The current snapshot; rejects when the application cannot give one (output filter unavailable). */
  getSnapshot(): Promise<AppSnapshot>
  /** Every later snapshot (`null`: one that could not be read); resolves to the function that stops listening. */
  onSnapshot(listener: (snapshot: AppSnapshot | null) => void): Promise<() => void>
  /** « Se connecter »: answers at once, the snapshot follows the login. */
  connectSession(session: string): Promise<void>
  /** « Déconnecter ». */
  disconnectSession(session: string): Promise<void>
  /** The profile chosen at first launch or confirmed in the change dialog. */
  setProfile(profile: string): Promise<void>
  /** « Reprendre » in the banner of the suspended gateway. */
  resumeGateway(): Promise<void>
  /** « Voir » in the banner of the waiting confirmations: their detail windows. Answers nothing. */
  showConfirmations(): Promise<void>
  /** A pastille of the MCP tab: exposes the environment to the LLM clients, or withdraws it. */
  setEnvironmentExposed(environment: string, exposed: boolean): Promise<void>
  /** The switch of a row of the MCP tab: cuts the MCP in every environment, or puts it back. */
  setMcpExposed(mcp: string, exposed: boolean): Promise<void>
  /** « J'ai vu » in the banner of an updated MCP. */
  acknowledgeMcpUpdate(environment: string, mcp: string): Promise<void>
  /** « Recharger la liste » of an exposed instance. */
  reloadMcp(environment: string, mcp: string): Promise<void>
  /** The entries of the journal, oldest first; rejects when the application cannot give them. */
  getJournal(): Promise<JournalEntry[]>
  /** Every later entry, or a resynchronisation; resolves to the function that stops listening. */
  onJournal(listener: (event: JournalEvent) => void): Promise<() => void>
  /** « Exporter un diagnostic »: the application asks where (its save dialog), then writes the archive there. */
  exportDiagnostic(): Promise<ExportOutcome>
  /** « Redémarrer pour mettre à jour », once confirmed: the application starts its new version, then stops. */
  restartForUpdate(): Promise<void>
  /** The dialog « Clients LLM »: the state of each configuration file. */
  getLlmClients(): Promise<ClientsView>
  /** What « Installer » or « Retirer » would change in one file; nothing is written. */
  previewLlmClient(location: LocationId, action: ClientAction): Promise<ClientPreview>
  /** The change shown by the preview whose fingerprint is sent back; `changed` if the file moved since. */
  applyLlmClient(location: LocationId, action: ClientAction, fingerprint: string): Promise<ApplyOutcome>
}

/** `get_journal` gave something that is not exactly a journal: nothing of it is shown. */
export class InvalidJournal extends Error {
  constructor() {
    super('journal illisible')
  }
}

/** The French reason of a refused command, as the application words it; a fixed text otherwise. */
export function reason(error: unknown): string {
  if (typeof error === 'string' && error.length > 0) return error
  if (error instanceof Error && error.message.length > 0) return error.message
  return 'erreur inattendue'
}
