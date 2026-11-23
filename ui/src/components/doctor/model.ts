// What the dialog « Auto-diagnostic » shows, as pure functions of the report of the application (spec 024).

import type { DoctorCheck, DoctorReport, DoctorStatus } from '@/api/types'
import { fr } from '@/i18n/fr'

/** Black and white for what is fine, orange for what waits for an action, red for what is broken. */
export function statusClass(status: DoctorStatus): string {
  switch (status) {
    case 'warning':
      return 'text-attention-text'
    case 'failure':
      return 'text-destructive-text'
    case 'ok':
      return 'text-muted-foreground'
  }
}

/** The line above the checks. */
export function summaryText(report: DoctorReport): string {
  switch (report.status) {
    case 'ok':
      return fr.doctor.summary.ok
    case 'warning':
      return fr.doctor.summary.warning(report.summary)
    case 'failure':
      return fr.doctor.summary.failure(report.summary)
  }
}

/** The same advice for several findings of a check is shown once, after the first, as in the text of the report. */
export function shownFix(check: DoctorCheck, index: number): string | null {
  const fix = check.findings[index]?.fix ?? null
  if (fix === null) return null
  return check.findings.slice(0, index).some((f) => f.fix === fix) ? null : fix
}

/** The mark of a finding, as in the text of the report. */
export function mark(status: DoctorStatus): string {
  switch (status) {
    case 'ok':
      return '–'
    case 'warning':
      return '!'
    case 'failure':
      return '×'
  }
}

/**
 * French spacing for the screen only: no line break inside « … » nor before « : », « ; », « ? », « ! ». The text
 * copied is the report as written.
 */
export function typography(text: string): string {
  return text
    .replaceAll('« ', '«\u00a0')
    .replaceAll(' »', '\u00a0»')
    .replace(/ ([:;?!])/g, '\u00a0$1')
}
