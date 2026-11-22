import { type Page, expect, test } from '@playwright/test'

// The installation window of spec 019 on `vite preview`, with the simulated installation and the CSP of the
// application, at the size the application gives it.
const CAPTURES = process.env.CAPTURES_DIR
const SIZE = { width: 500, height: 440 }

async function open(page: Page, scenario: string, colorScheme: 'light' | 'dark'): Promise<string[]> {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', (error) => errors.push(error.message))
  await page.emulateMedia({ colorScheme })
  await page.setViewportSize(SIZE)
  await page.goto(`/install.html?scenario=${scenario}`)
  return errors
}

/** Nothing overflows the window: no scroll bar in a window that cannot be resized. */
async function fits(page: Page): Promise<boolean> {
  return page.evaluate(() => document.documentElement.scrollHeight <= window.innerHeight)
}

for (const colorScheme of ['light', 'dark'] as const) {
  // Exigence : EX-019-13
  test(`installation, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'installation', colorScheme)
    await expect(page.getByRole('heading', { name: 'Installer MCP Proxy' })).toBeVisible()
    const box = page.getByRole('checkbox', { name: 'Démarrer MCP Proxy avec Windows' })
    await expect(box).not.toBeChecked()
    await box.check()
    expect(await fits(page)).toBe(true)
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/installation-${colorScheme}.png` })
    await page.getByRole('button', { name: 'Installer' }).click()
    await expect(page.getByText('Installation en cours…')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Installer' })).toBeDisabled()
    expect(errors).toEqual([])
  })

  // Exigence : EX-019-13
  test(`update offered, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'mise-a-jour', colorScheme)
    await expect(page.getByRole('heading', { name: 'Mettre à jour MCP Proxy' })).toBeVisible()
    await expect(page.getByText('La version 0.1.0 est installée. Cet exe est la version 0.2.0.')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Lancer la version 0.1.0' })).toBeEnabled()
    expect(await fits(page)).toBe(true)
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/mise-a-jour-${colorScheme}.png` })
    expect(errors).toEqual([])
  })

  // Exigence : EX-019-13
  test(`failure, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'installation-echec', colorScheme)
    await page.getByRole('button', { name: 'Installer' }).click()
    await expect(page.getByRole('heading', { name: "L'installation n'a pas abouti" })).toBeVisible()
    await expect(page.getByRole('alert')).toContainText('accès refusé')
    await expect(page.getByRole('button', { name: 'Continuer sans installer' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Quitter' })).toBeVisible()
    expect(await fits(page)).toBe(true)
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/echec-installation-${colorScheme}.png` })
    expect(errors).toEqual([])
  })

  // Exigence : EX-019-13
  test(`update failure, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'mise-a-jour-echec', colorScheme)
    await page.getByRole('button', { name: 'Mettre à jour' }).click()
    await expect(page.getByRole('heading', { name: "La mise à jour n'a pas abouti" })).toBeVisible()
    await expect(page.getByRole('alert')).toContainText("n'a pas pu être fermé")
    expect(await fits(page)).toBe(true)
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/echec-mise-a-jour-${colorScheme}.png` })
    expect(errors).toEqual([])
  })
}
