// Simulated installation, built only in `--mode mock`: the data the application would give and its answers, by
// scenario (`?scenario=…`). `scripts/check-dist.mjs` fails the application's build if it contains it.

import type { InstallData, Send } from './install'

/** Marker searched for in the application's build by `scripts/check-dist.mjs`. */
export const INSTALL_MOCK_MARKER = 'mcp-proxy-simulated-installer'

export const INSTALL_SCENARIOS = ['installation', 'installation-echec', 'mise-a-jour', 'mise-a-jour-echec'] as const
export type InstallScenario = (typeof INSTALL_SCENARIOS)[number]

const FAILURES: Record<'install' | 'update', string> = {
  install:
    "Création du dossier d'installation impossible (accès refusé). Rien n'a été modifié : tout ce qui avait été fait a été retiré.",
  update:
    "MCP Proxy, déjà ouvert, n'a pas pu être fermé (toujours ouvert après 20 s). Rien n'a été modifié. Quittez MCP Proxy depuis son icône, puis réessayez.",
}

export function scenarioOf(search: string): InstallScenario {
  const name = new URLSearchParams(search).get('scenario')
  return INSTALL_SCENARIOS.find((s) => s === name) ?? 'installation'
}

export function simulated(search: string): { data: InstallData; send: Send } {
  const scenario = scenarioOf(search)
  const update = scenario.startsWith('mise-a-jour')
  const data: InstallData = {
    kind: update ? 'update' : 'install',
    version: '0.2.0',
    installedVersion: update ? '0.1.0' : null,
    autostart: false,
    location: '%LOCALAPPDATA%\\Programs\\MCP Proxy',
    command: 'answer_install',
  }
  const fails = scenario.endsWith('echec')
  const send: Send = async (answer) => {
    await new Promise((resolve) => setTimeout(resolve, 300))
    const acting = answer.choice === 'install' || answer.choice === 'update'
    if (fails && acting) throw new Error(FAILURES[data.kind])
    // Success: the application closes the window; the page stays as it is meanwhile.
  }
  return { data, send }
}
