import { type Page, expect, test } from '@playwright/test'

// The uninstallation window of spec 022 on `vite preview`, with the simulated uninstallation and the CSP of the
// application, at the size the application gives it.
const CAPTURES = process.env.CAPTURES_DIR
const SIZE = { width: 560, height: 700 }

async function open(page: Page, scenario: string, colorScheme: 'light' | 'dark'): Promise<string[]> {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', (error) => errors.push(error.message))
  await page.emulateMedia({ colorScheme })
  await page.setViewportSize(SIZE)
  await page.goto(`/uninstall.html?scenario=${scenario}`)
  return errors
}

/** Nothing overflows the window: no scroll bar in a window that cannot be resized, no text cut on the right. */
async function fits(page: Page): Promise<boolean> {
  return page.evaluate(
    () =>
      document.documentElement.scrollHeight <= window.innerHeight &&
      document.documentElement.scrollWidth <= window.innerWidth,
  )
}

/** The lists scroll inside the window when they are longer than it: the buttons stay visible. */
async function buttonsVisible(page: Page, name: string): Promise<void> {
  const button = page.getByRole('button', { name, exact: true })
  await expect(button).toBeInViewport()
}

for (const colorScheme of ['light', 'dark'] as const) {
  // Exigence : EX-022-11
  test(`confirmation, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'desinstallation', colorScheme)
    await expect(page.getByRole('heading', { name: 'Désinstaller MCP Proxy' })).toBeVisible()
    await expect(page.getByRole('checkbox', { name: 'Retirer MCP Proxy des clients LLM' })).toBeChecked()
    await expect(page.getByText('les journaux, les réglages', { exact: false })).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Ne sera pas supprimé' })).toBeVisible()
    expect(await fits(page)).toBe(true)
    await buttonsVisible(page, 'Désinstaller')
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/confirmation-${colorScheme}.png` })
    expect(errors).toEqual([])
  })

  // Exigence : EX-022-11
  test(`box unticked, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'desinstallation', colorScheme)
    await page.getByRole('checkbox', { name: 'Retirer MCP Proxy des clients LLM' }).uncheck()
    await expect(page.getByText('Les fichiers des clients LLM ne sont pas modifiés', { exact: false })).toBeVisible()
    expect(await fits(page)).toBe(true)
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/case-decochee-${colorScheme}.png` })
    await page.getByRole('button', { name: 'Désinstaller', exact: true }).click()
    await expect(page.getByRole('heading', { name: 'MCP Proxy est désinstallé' })).toBeVisible()
    await expect(page.getByText('laissée, comme demandé', { exact: false })).toBeVisible()
    expect(errors).toEqual([])
  })

  // Exigence : EX-022-11
  test(`complete report, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'desinstallation', colorScheme)
    await page.getByRole('button', { name: 'Désinstaller', exact: true }).click()
    await expect(page.getByText('Désinstallation en cours…')).toBeVisible()
    await expect(page.getByRole('heading', { name: 'MCP Proxy est désinstallé' })).toBeVisible()
    await expect(page.getByText('Programme mcp-proxy.exe : à la fermeture de cette fenêtre')).toBeVisible()
    expect(await fits(page)).toBe(true)
    await buttonsVisible(page, 'Fermer')
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/compte-rendu-complet-${colorScheme}.png` })
    expect(errors).toEqual([])
  })

  // Exigence : EX-022-11
  test(`report with leftovers, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'desinstallation-restes', colorScheme)
    await page.getByRole('button', { name: 'Désinstaller', exact: true }).click()
    await expect(page.getByRole('heading', { name: 'Désinstallation incomplète' })).toBeVisible()
    await expect(page.getByText('fichier utilisé par un autre programme', { exact: false })).toBeVisible()
    await expect(page.getByText('relancez la désinstallation', { exact: false })).toBeVisible()
    expect(await fits(page)).toBe(true)
    await buttonsVisible(page, 'Fermer')
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/compte-rendu-restes-${colorScheme}.png` })
    expect(errors).toEqual([])
  })

  // Exigence : EX-022-11
  test(`nothing done, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'desinstallation-echec', colorScheme)
    await page.getByRole('button', { name: 'Désinstaller', exact: true }).click()
    await expect(page.getByRole('heading', { name: "La désinstallation n'a pas eu lieu" })).toBeVisible()
    await expect(page.getByRole('alert')).toContainText('Rien n\'a été modifié')
    expect(await fits(page)).toBe(true)
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/echec-${colorScheme}.png` })
    await page.getByRole('button', { name: 'Réessayer' }).click()
    await expect(page.getByRole('heading', { name: 'MCP Proxy est désinstallé' })).toBeVisible()
    expect(errors).toEqual([])
  })
}
