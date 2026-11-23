import { type Page, expect, test } from '@playwright/test'

// The menu of the header and the dialog « Clients LLM » (spec 023) on `vite preview`, with the simulated backend and
// the CSP of the application.
const NOW = new Date('2026-11-22T14:03:07Z')
const CAPTURES = process.env.CAPTURES_DIR

async function open(page: Page, scenario: string, colorScheme: 'light' | 'dark'): Promise<string[]> {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', (error) => errors.push(error.message))
  await page.emulateMedia({ colorScheme })
  await page.clock.setFixedTime(NOW)
  await page.goto(`/index.html?scenario=${scenario}`)
  await expect(page.locator('header h1')).toHaveText('MCP Proxy')
  return errors
}

async function settled(page: Page) {
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running'))
}

async function capture(page: Page, name: string) {
  await settled(page)
  if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/${name}.png` })
}

async function openDialog(page: Page) {
  await page.locator('[data-action="app-menu"]').click()
  await page.locator('[data-menu="clients"]').click()
  await expect(page.locator('[data-dialog="clients"] [data-location]')).toHaveCount(3)
}

/** The dialog stays inside the window, and nothing in it is wider than it. */
async function fits(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const dialog = document.querySelector('[data-dialog="clients"]')
    if (!dialog) return ['no dialog']
    const bounds = dialog.getBoundingClientRect()
    const problems: string[] = []
    if (bounds.left < 0 || bounds.right > window.innerWidth || bounds.top < 0 || bounds.bottom > window.innerHeight)
      problems.push('dialog out of the window')
    for (const element of dialog.querySelectorAll<HTMLElement>('p, li, button, code')) {
      const r = element.getBoundingClientRect()
      if (r.width > 0 && (r.left < bounds.left - 1 || r.right > bounds.right + 1))
        problems.push(`out: ${element.textContent.slice(0, 40)}`)
    }
    return problems
  })
}

/**
 * Elements drawn with the orange or the red of the theme (text, background or border), by their row and text. The
 * reference colors are those of the tokens, read from probe elements.
 */
async function colored(page: Page): Promise<{ orange: string[]; red: string[] }> {
  return page.evaluate(() => {
    // Through the CSSOM, which the CSP allows (unlike a style attribute in the markup).
    const probe = (tokens: string[]) =>
      new Set(
        tokens.map((token) => {
          const span = document.createElement('span')
          span.style.color = `var(${token})`
          document.body.append(span)
          const color = getComputedStyle(span).color
          span.remove()
          return color
        }),
      )
    const orange = probe(['--attention', '--attention-text'])
    const red = probe(['--destructive', '--destructive-text'])
    const found = { orange: new Set<string>(), red: new Set<string>() }
    for (const element of document.querySelectorAll<HTMLElement>('[data-dialog="clients"] *')) {
      if (element.getClientRects().length === 0 || element.childElementCount > 0) continue
      const style = getComputedStyle(element)
      const where = `${element.closest<HTMLElement>('[data-location]')?.dataset.location ?? '-'}: ${element.textContent.trim().slice(0, 30)}`
      for (const color of [style.color, style.backgroundColor, style.borderTopColor]) {
        if (orange.has(color)) found.orange.add(where)
        if (red.has(color)) found.red.add(where)
      }
    }
    return { orange: [...found.orange], red: [...found.red] }
  })
}

for (const colorScheme of ['light', 'dark'] as const) {
  // Exigence : EX-023-16
  test(`menu, liste et installation, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'clients-etats', colorScheme)
    await page.locator('[data-action="app-menu"]').click()
    await expect(page.locator('[data-menu="clients"]')).toHaveText('Clients LLM…')
    await capture(page, `menu-${colorScheme}`)
    await page.locator('[data-menu="clients"]').click()
    const dialog = page.locator('[data-dialog="clients"]')
    await expect(dialog.locator('[data-location]')).toHaveCount(3)
    await expect(dialog.locator('[data-location="claude-desktop"] [data-client-state]')).toHaveText(
      'configuré avec un autre chemin',
    )
    await expect(dialog.locator('[data-location="claude-desktop-msix"] [data-client-state]')).toHaveText('non installé')
    await expect(dialog.locator('[data-location="vscode"] [data-client-state]')).toHaveText('non configuré')
    expect(await fits(page)).toEqual([])
    // Orange only for the entry that waits for « Installer »; no red without an error.
    expect(await colored(page)).toEqual({ orange: ['claude-desktop: configuré avec un autre chemin'], red: [] })
    await capture(page, `liste-etats-${colorScheme}`)

    await dialog.locator('[data-location="claude-desktop"] [data-action="client-install"]').click()
    await expect(dialog).toContainText('Ajouter MCP Proxy à Claude Desktop ?')
    await expect(dialog.locator('[data-diff]')).toContainText('- "mcp-proxy": { "command": "C:\\\\Users\\\\exemple\\\\Downloads')
    await expect(dialog.locator('[data-diff]')).toContainText('+   "args": ["stdio"]')
    await expect(dialog).toContainText('Les 2 autres serveurs et le reste du fichier ne sont pas modifiés.')
    expect(await fits(page)).toEqual([])
    await capture(page, `apercu-${colorScheme}`)

    await dialog.locator('[data-action="client-confirm"]').click()
    await expect(dialog).toContainText('MCP Proxy a été ajouté à Claude Desktop.')
    await expect(dialog).toContainText('claude_desktop_config.json.bak-20261122-140307')
    await expect(dialog.locator('[data-notice="restart"]')).toContainText('zone de notification')
    await capture(page, `apres-installation-${colorScheme}`)

    await dialog.locator('[data-action="client-back"]').click()
    await expect(dialog.locator('[data-location="claude-desktop"]')).toHaveAttribute('data-state', 'configured')
    await dialog.locator('[data-action="client-close"]').click()
    await expect(dialog).toHaveCount(0)
    // The journal of the window says what the user did, as the gateway does.
    await page.getByRole('tab', { name: 'Journal' }).click()
    await expect(page.locator('main')).toContainText("MCP Proxy ajouté à la configuration de Claude Desktop par l'utilisateur")
    expect(errors).toEqual([])
  })

  // Exigence : EX-023-16
  test(`fichier illisible : extrait à coller, ${colorScheme}`, async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write'])
    const errors = await open(page, 'clients-illisible', colorScheme)
    await openDialog(page)
    const dialog = page.locator('[data-dialog="clients"]')
    await expect(dialog.locator('[data-location="claude-desktop"] [data-client-state]')).toHaveText('fichier illisible')
    await capture(page, `illisible-liste-${colorScheme}`)
    await dialog.locator('[data-location="claude-desktop"] [data-action="client-install"]').click()
    await expect(dialog).toContainText('ne peut pas être modifié sans risque (JSON invalide à la ligne 12)')
    await expect(dialog.locator('[data-action="client-confirm"]')).toHaveCount(0)
    await dialog.locator('[data-action="client-copy-entry"]').click()
    await expect(dialog.locator('[data-copy]')).toContainText('Configuration copiée')
    const copied = await page.evaluate(() => navigator.clipboard.readText())
    expect(copied).toContain('"mcpServers"')
    expect(JSON.parse(copied)).toEqual({
      mcpServers: {
        'mcp-proxy': {
          command: 'C:\\Users\\exemple\\AppData\\Local\\Programs\\MCP Proxy\\mcp-proxy.exe',
          args: ['stdio'],
        },
      },
    })
    expect(await fits(page)).toEqual([])
    await capture(page, `illisible-extrait-${colorScheme}`)
    expect(errors).toEqual([])
  })

  // Exigence : EX-023-16
  test(`deux emplacements de Claude Desktop, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'clients-deux-emplacements', colorScheme)
    await openDialog(page)
    const dialog = page.locator('[data-dialog="clients"]')
    await expect(dialog.locator('[data-notice="two-locations"]')).toContainText('deux fichiers de configuration')
    await expect(dialog.locator('[data-location="claude-desktop"]')).toContainText('Sans doute pas lu')
    await expect(dialog.locator('[data-location="claude-desktop-msix"] [data-client-state]')).toHaveText('configuré')
    expect(await fits(page)).toEqual([])
    expect((await colored(page)).red).toEqual([])
    await capture(page, `deux-emplacements-${colorScheme}`)
    await dialog.locator('[data-location="claude-desktop-msix"] [data-action="client-remove"]').click()
    await expect(dialog).toContainText('Retirer MCP Proxy de Claude Desktop (paquet MSIX) ?')
    await capture(page, `deux-emplacements-retrait-${colorScheme}`)
    await dialog.locator('[data-action="client-confirm"]').click()
    await expect(dialog).toContainText('MCP Proxy a été retiré de Claude Desktop (paquet MSIX).')
    expect(errors).toEqual([])
  })

  // Exigence : EX-023-16
  test(`MCP Proxy non installé, fichier modifié entre-temps, ${colorScheme}`, async ({ page }) => {
    let errors = await open(page, 'clients-non-installe', colorScheme)
    await openDialog(page)
    const dialog = page.locator('[data-dialog="clients"]')
    await expect(dialog.locator('[data-notice="not-installed"]')).toContainText("MCP Proxy n'est pas installé")
    await expect(dialog.locator('[data-action="client-copy"]')).toHaveCount(0)
    await expect(dialog.locator('[data-location="vscode"] [data-action="client-install"]')).toBeDisabled()
    await expect(dialog.locator('[data-location="claude-desktop"] [data-action="client-remove"]')).toBeEnabled()
    await capture(page, `non-installe-${colorScheme}`)
    expect(errors).toEqual([])

    errors = await open(page, 'clients-modifie', colorScheme)
    await openDialog(page)
    await dialog.locator('[data-location="vscode"] [data-action="client-install"]').click()
    await dialog.locator('[data-action="client-confirm"]').click()
    await expect(dialog.locator('[data-notice="changed"]')).toContainText("Le fichier a changé depuis l'aperçu")
    await capture(page, `modifie-${colorScheme}`)
    await dialog.locator('[data-action="client-confirm"]').click()
    await expect(dialog).toContainText('MCP Proxy a été ajouté à VS Code.')
    expect(errors).toEqual([])
  })
}
