import { type Page, expect, test } from '@playwright/test'

// « Redémarrer pour mettre à jour » (spec 020) on `vite preview`, with the simulated backend and the CSP of the
// application.
const NOW = new Date('2026-11-22T14:00:00Z')
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

/** The parts of the header stay inside it, side by side. */
async function headerFits(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const header = document.querySelector('header > div')
    if (!header) return ['no header']
    const bounds = header.getBoundingClientRect()
    const selectors = ['h1', 'h1 + span', '[data-update-critical]', '[data-action="update"]', '[data-action="profile-menu"]']
    const parts = selectors
      .map((selector) => header.querySelector<HTMLElement>(selector))
      .filter((e): e is HTMLElement => e !== null)
    const problems: string[] = []
    for (const [i, part] of parts.entries()) {
      const r = part.getBoundingClientRect()
      if (r.left < bounds.left || r.right > bounds.right || r.top < bounds.top || r.bottom > bounds.bottom)
        problems.push(`out: ${part.textContent}`)
      for (const other of parts.slice(i + 1)) {
        const o = other.getBoundingClientRect()
        if (r.left < o.right && o.left < r.right && r.top < o.bottom && o.top < r.bottom)
          problems.push(`overlap: ${part.textContent} / ${other.textContent}`)
      }
    }
    return problems
  })
}

/** Once the dialog has finished appearing: a capture taken during its animation shows it half transparent. */
async function settled(page: Page) {
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running'))
}

for (const colorScheme of ['light', 'dark'] as const) {
  // Exigence : EX-020-18
  test(`mise à jour prête, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'mise-a-jour-prete', colorScheme)
    const button = page.locator('[data-action="update"]')
    await expect(button).toBeEnabled()
    await expect(button).toHaveText('Redémarrer pour mettre à jour')
    await expect(page.locator('[data-update-critical]')).toHaveCount(0)
    expect(await headerFits(page)).toEqual([])
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/mise-a-jour-prete-${colorScheme}.png` })

    await button.click()
    const dialog = page.locator('[data-dialog="restart"]')
    await expect(dialog).toBeVisible()
    await expect(dialog).toContainText('Redémarrer pour passer à MCP Proxy 0.2.0 ?')
    await expect(dialog).toContainText('il faudra vous reconnecter')
    await settled(page)
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/mise-a-jour-dialogue-${colorScheme}.png` })
    await dialog.getByRole('button', { name: 'Annuler' }).click()
    await expect(dialog).toHaveCount(0)

    const restart = page.waitForEvent('console', (m) => m.text().includes('restart_for_update'))
    await button.click()
    await page.locator('[data-action="confirm-restart"]').click()
    await restart
    await expect(page.locator('[data-dialog="restart"]')).toHaveCount(0)
    expect(errors).toEqual([])
  })

  // Exigence : EX-020-18
  test(`mise à jour critique, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'mise-a-jour-critique', colorScheme)
    await expect(page.locator('[data-update-critical]')).toHaveText('Mise à jour critique')
    await expect(page.locator('[data-action="update"]')).toBeEnabled()
    expect(await headerFits(page)).toEqual([])
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/mise-a-jour-critique-${colorScheme}.png` })
    await page.locator('[data-action="update"]').click()
    await expect(page.locator('[data-dialog="restart"]')).toContainText('il redémarrera de lui-même')
    await settled(page)
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/mise-a-jour-critique-dialogue-${colorScheme}.png` })
    expect(errors).toEqual([])
  })
}
