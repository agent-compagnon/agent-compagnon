// Simulated uninstallation, built only in `--mode mock`: the data the application would give and its answers, by
// scenario (`?scenario=…`), taken from the contract written by the `deploy` crate. `scripts/check-dist.mjs` fails
// the application's build if it contains it.

import example from './uninstall.example.json'
import { type Send, type UninstallData, type UninstallReport, isData, isReport } from './uninstall'

/** Marker searched for in the application's build by `scripts/check-dist.mjs`. */
export const UNINSTALL_MOCK_MARKER = 'mcp-proxy-simulated-uninstaller'

export const UNINSTALL_SCENARIOS = [
  'desinstallation',
  'desinstallation-restes',
  'desinstallation-echec',
  'desinstallation-sans-client',
] as const
export type UninstallScenario = (typeof UNINSTALL_SCENARIOS)[number]

export function scenarioOf(search: string): UninstallScenario {
  const name = new URLSearchParams(search).get('scenario')
  return UNINSTALL_SCENARIOS.find((s) => s === name) ?? 'desinstallation'
}

function checked<T>(value: unknown, check: (value: unknown) => value is T): T {
  if (!check(value)) throw new Error('uninstall.example.json ne suit plus le contrat de la fenêtre')
  return value
}

export function simulated(search: string): { data: UninstallData; send: Send } {
  const scenario = scenarioOf(search)
  const base = checked(example.data, isData)
  const data: UninstallData =
    scenario === 'desinstallation-sans-client'
      ? { ...base, clients: [] }
      : base
  const report: UninstallReport = checked(
    scenario === 'desinstallation-restes' ? example.withLeftovers : example.complete,
    isReport,
  )
  let failures = scenario === 'desinstallation-echec' ? 1 : 0
  const send: Send = async (answer) => {
    await new Promise((resolve) => setTimeout(resolve, 300))
    if (answer.choice !== 'uninstall') return null
    if (failures > 0) {
      failures -= 1
      throw new Error(example.notStopped)
    }
    if (answer.removeClients) return report
    // The box unticked: the entries the report says removed are left instead, as the application would.
    const entries = report.removed.filter((item) => item.startsWith('Entrée « mcp-proxy »'))
    return {
      ...report,
      removed: report.removed.filter((item) => !entries.includes(item)),
      left: [...entries.map((what) => ({ what, why: 'laissée, comme demandé', failed: false })), ...report.left],
    }
  }
  return { data, send }
}
