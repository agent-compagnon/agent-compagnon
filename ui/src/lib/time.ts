// Times of the snapshot are instants (`at`, milliseconds since the Unix epoch); the labels of the core are in UTC.
// The window shows the local time of Windows.

import { fr } from '@/i18n/fr'

const hourMinute = new Intl.DateTimeFormat('fr-FR', { hour: '2-digit', minute: '2-digit' })
const dayHourMinute = new Intl.DateTimeFormat('fr-FR', {
  day: '2-digit',
  month: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
})

function dayOf(at: number): string {
  return new Date(at).toDateString()
}

/** « active jusqu'à 17:42 », « … jusqu'à demain 09:10 », or with the date beyond tomorrow; local time. */
export function activeUntil(at: number, now: number): string {
  if (dayOf(at) === dayOf(now)) return fr.connections.state.active(hourMinute.format(at))
  const tomorrow = new Date(now)
  tomorrow.setDate(tomorrow.getDate() + 1)
  if (dayOf(at) === tomorrow.toDateString()) return fr.connections.state.activeTomorrow(hourMinute.format(at))
  return fr.connections.state.active(dayHourMinute.format(at))
}
