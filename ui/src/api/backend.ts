// What the main window asks of the application. Two implementations: `tauri.ts` (the application, through the
// commands granted to the window `main`) and `mock.ts` (a simulated backend, built only in `--mode mock`, to run
// the window in a plain browser). The window never holds anything secret: it shows the snapshot and sends
// identifiers.

import type { AppSnapshot } from './types'

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
}

/** The French reason of a refused command, as the application words it; a fixed text otherwise. */
export function reason(error: unknown): string {
  if (typeof error === 'string' && error.length > 0) return error
  if (error instanceof Error && error.message.length > 0) return error.message
  return 'erreur inattendue'
}
