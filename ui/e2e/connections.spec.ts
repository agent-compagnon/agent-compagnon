import { type Page, expect, test } from '@playwright/test'

// The main window on `vite preview`, with the simulated backend and the CSP of the application. Sun, 22 Nov 2026
// 14:00 UTC (15:00 in Paris): the times of the scenarios are deterministic.
const NOW = new Date('2026-11-22T14:00:00Z')
const CAPTURES = process.env.CAPTURES_DIR

const SCENARIOS = [
  'premier-lancement',
  'sessions',
  'connexion-demandee',
  'confirmation',
  'suspendue',
  'reglages-reinitialises',
  'profil-webview',
  'mcp-mis-a-jour',
] as const

/** Opens a scenario; returns the errors of the page, CSP violations included. */
async function open(page: Page, scenario: string, colorScheme: 'light' | 'dark' = 'light'): Promise<string[]> {
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

/**
 * Every visible element drawn in orange (text, background or border), outside of what waits for the user: a login
 * banner, a confirmation banner, a card whose login is asked. Orange: hue between 20° and 80° with some chroma.
 */
async function strayOrange(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const canvas = document.createElement('canvas').getContext('2d')
    function hue(color: string): { hue: number; chroma: number; alpha: number } | null {
      if (!canvas || color === '' || color === 'transparent') return null
      canvas.fillStyle = '#000'
      canvas.fillStyle = color
      canvas.clearRect(0, 0, 1, 1)
      canvas.fillRect(0, 0, 1, 1)
      const [r = 0, g = 0, b = 0, a = 0] = canvas.getImageData(0, 0, 1, 1).data
      const max = Math.max(r, g, b)
      const min = Math.min(r, g, b)
      const chroma = (max - min) / 255
      if (chroma === 0) return { hue: 0, chroma, alpha: a / 255 }
      let h = max === r ? ((g - b) / (max - min)) % 6 : max === g ? (b - r) / (max - min) + 2 : (r - g) / (max - min) + 4
      h *= 60
      if (h < 0) h += 360
      return { hue: h, chroma, alpha: a / 255 }
    }
    const allowed = '[data-banner="login"], [data-banner="confirmations"], [data-state="loginRequested"]'
    const found: string[] = []
    for (const element of document.querySelectorAll<HTMLElement>('body *')) {
      if (element.closest(allowed) || element.getClientRects().length === 0) continue
      const style = getComputedStyle(element)
      for (const color of [style.color, style.backgroundColor, style.borderTopColor]) {
        const c = hue(color)
        if (c && c.alpha > 0.05 && c.chroma > 0.15 && c.hue >= 20 && c.hue <= 80) {
          found.push(`${element.tagName.toLowerCase()}.${String(element.getAttribute('class')).slice(0, 60)}: ${color}`)
        }
      }
    }
    return found
  })
}

for (const scenario of SCENARIOS) {
  for (const colorScheme of ['light', 'dark'] as const) {
    // Exigence : EX-016-02
    test(`Connexions, ${scenario}, ${colorScheme}`, async ({ page }) => {
      const errors = await open(page, scenario, colorScheme)
      await page.waitForTimeout(250)
      if (CAPTURES) {
        await page.screenshot({ path: `${CAPTURES}/connexions-${scenario}-${colorScheme}.png`, fullPage: true })
      }
      expect(errors, 'no CSP violation nor error').toEqual([])
      expect(await strayOrange(page), 'orange only on what waits for the user').toEqual([])
    })
  }
}

// Exigence : EX-016-02
test('the orange marks exactly what waits for the user', async ({ page }) => {
  await open(page, 'connexion-demandee')
  await expect(page.locator('[data-banner="login"]')).toContainText('Claude Desktop attend une connexion à SSO PROD.')
  await expect(page.locator('[data-state="loginRequested"]')).toHaveCount(1)
  await open(page, 'confirmation')
  await expect(page.locator('[data-banner="confirmations"]')).toContainText('Une confirmation attend votre réponse.')
  await expect(page.locator('[data-state="loginRequested"]')).toHaveCount(0)
})

// Exigence : EX-016-03
test('works with the keyboard alone: profile menu, tabs, cards', async ({ page }) => {
  await open(page, 'sessions')
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Profil : Développeur' })).toBeFocused()
  await page.keyboard.press('Enter')
  const menu = page.getByRole('menu')
  await expect(menu).toBeVisible()
  await expect(menu.getByRole('menuitemradio', { name: /Développeur/ })).toHaveAttribute('aria-checked', 'true')
  await page.keyboard.press('Escape')
  await expect(menu).toBeHidden()
  await expect(page.getByRole('button', { name: 'Profil : Développeur' })).toBeFocused()

  await page.keyboard.press('Tab')
  await expect(page.getByRole('tab', { name: 'Connexions' })).toBeFocused()
  await page.keyboard.press('ArrowRight')
  await expect(page.getByRole('tab', { name: 'MCP' })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('tab', { name: 'MCP' })).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('ArrowLeft')
  await page.keyboard.press('Enter')
  await expect(page.getByRole('tab', { name: 'Connexions' })).toHaveAttribute('aria-selected', 'true')

  // Tab leaves the tab list for the panel, then reaches the first card's button.
  await page.keyboard.press('Tab')
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Déconnecter SSO Entreprise' })).toBeFocused()
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Se connecter à SSO Outils' })).toBeFocused()
  // Focus is visible.
  const ring = await page
    .getByRole('button', { name: 'Se connecter à SSO Outils' })
    .evaluate((el) => getComputedStyle(el).boxShadow)
  expect(ring).not.toBe('none')
})

// Exigence : EX-016-03
test('the first-launch dialog is chosen with the keyboard, and cannot be escaped', async ({ page }) => {
  await open(page, 'premier-lancement')
  const dialog = page.getByRole('alertdialog')
  await expect(dialog).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(dialog).toBeVisible()
  await expect(dialog.getByRole('button', { name: 'Continuer' })).toBeDisabled()
  // Focus is kept inside the dialog; the radio group is reached and moved through with the keyboard.
  await expect.poll(() => page.evaluate(() => document.activeElement?.closest('[role="alertdialog"]') !== null)).toBe(true)
  await dialog.getByRole('radio', { name: 'Développeur' }).focus()
  await page.keyboard.press('ArrowDown')
  await expect(dialog.getByRole('radio', { name: 'Support' })).toBeFocused()
  await page.keyboard.press('Space')
  await expect(dialog.getByRole('radio', { name: 'Support' })).toBeChecked()
  await page.keyboard.press('Tab')
  await expect(dialog.getByRole('button', { name: 'Continuer' })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(dialog).toBeHidden()
  await expect(page.getByRole('button', { name: 'Profil : Support' })).toBeVisible()
  await expect(page.locator('[data-session="sso-prod"]')).toContainText('jamais connectée')
})

// Exigence : EX-016-01
test('a card follows its login, then its disconnection', async ({ page }) => {
  await open(page, 'sessions')
  const card = page.locator('[data-session="sso-int"]')
  await card.getByRole('button', { name: 'Se connecter à SSO INT' }).click()
  await expect(card).toHaveAttribute('data-state', 'loginRequested')
  await expect(card).toContainText('connexion demandée')
  await expect(page.locator('[data-banner="login"]'), 'asked from the window: no banner').toHaveCount(0)
  await expect(card).toHaveAttribute('data-state', 'active')
  await expect(card).toContainText("active jusqu'à 23:00")
  await card.getByRole('button', { name: 'Déconnecter SSO INT' }).click()
  await expect(card).toHaveAttribute('data-state', 'expired')
  await expect(card).toContainText('expirée')
})

// Exigence : EX-016-01
test('changing the profile says its effect first', async ({ page }) => {
  await open(page, 'sessions')
  await page.getByRole('button', { name: 'Profil : Développeur' }).click()
  await page.getByRole('menuitemradio', { name: /Support/ }).click()
  const dialog = page.getByRole('alertdialog')
  await expect(dialog).toContainText('Passer au profil « Support » ?')
  await expect(dialog).toContainText('REC, PPROD et PROD')
  await expect(dialog).toContainText('Vos durcissements sont conservés.')
  if (CAPTURES) {
    await page.waitForTimeout(400)
    await page.screenshot({ path: `${CAPTURES}/changement-de-profil-light.png` })
  }
  await dialog.getByRole('button', { name: 'Annuler' }).click()
  await expect(page.getByRole('button', { name: 'Profil : Développeur' })).toBeVisible()

  await page.getByRole('button', { name: 'Profil : Développeur' }).click()
  await page.getByRole('menuitemradio', { name: /Support/ }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: 'Changer de profil' }).click()
  await expect(page.getByRole('button', { name: 'Profil : Support' })).toBeVisible()
  await expect(page.locator('[data-session="sso-prod"]')).toBeVisible()
  await expect(page.locator('[data-session="sso-dev"]')).toHaveCount(0)
})

// Exigence : EX-016-01
test('« Reprendre » lifts the suspension banner', async ({ page }) => {
  await open(page, 'suspendue')
  await page.locator('[data-banner="suspended"]').getByRole('button', { name: 'Reprendre' }).click()
  await expect(page.locator('[data-banner="suspended"]')).toHaveCount(0)
})

test('the profile menu, open, in both themes', async ({ page }) => {
  if (!CAPTURES) return
  for (const scheme of ['light', 'dark'] as const) {
    await open(page, 'sessions', scheme)
    await page.getByRole('button', { name: 'Profil : Développeur' }).click()
    await expect(page.getByRole('menu')).toBeVisible()
    await page.waitForTimeout(400)
    await page.screenshot({ path: `${CAPTURES}/menu-profil-${scheme}.png` })
  }
})
