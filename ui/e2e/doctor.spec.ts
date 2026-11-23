import { type Page, expect, test } from '@playwright/test'

// The dialog « Auto-diagnostic » (spec 024) on `vite preview`, with the simulated backend (the reports of
// `doctor.example.json`, written by the gateway) and the CSP of the application.
const NOW = new Date('2026-11-23T10:00:00Z')
const CAPTURES = process.env.CAPTURES_DIR
const DIALOG = '[data-dialog="doctor"]'

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
  await page.locator('[data-menu="doctor"]').click()
  await expect(page.locator(DIALOG)).toBeVisible()
}

async function done(page: Page) {
  await expect(page.locator(DIALOG)).toHaveAttribute('data-phase', 'done')
  await expect(page.locator(`${DIALOG} [data-check]`)).toHaveCount(11)
}

/** The dialog stays inside the window, and nothing in it is wider than it. */
async function fits(page: Page): Promise<string[]> {
  return page.evaluate((selector) => {
    const dialog = document.querySelector(selector)
    if (!dialog) return ['no dialog']
    const bounds = dialog.getBoundingClientRect()
    const problems: string[] = []
    if (bounds.left < 0 || bounds.right > window.innerWidth || bounds.top < 0 || bounds.bottom > window.innerHeight)
      problems.push('dialog out of the window')
    for (const element of dialog.querySelectorAll<HTMLElement>('p, li, button, code, span')) {
      const r = element.getBoundingClientRect()
      if (r.width > 0 && (r.left < bounds.left - 1 || r.right > bounds.right + 1))
        problems.push(`out: ${element.textContent.slice(0, 40)}`)
    }
    return problems
  }, DIALOG)
}

/** The checks drawn in orange or in red (text, background or border), with the colors of the theme's tokens. */
async function colored(page: Page): Promise<{ orange: string[]; red: string[] }> {
  return page.evaluate((selector) => {
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
    for (const element of document.querySelectorAll<HTMLElement>(`${selector} *`)) {
      if (element.getClientRects().length === 0 || element.childElementCount > 0) continue
      const style = getComputedStyle(element)
      const where = element.closest<HTMLElement>('[data-check]')?.dataset.check ?? 'summary'
      for (const color of [style.color, style.backgroundColor, style.borderTopColor]) {
        if (orange.has(color)) found.orange.add(where)
        if (red.has(color)) found.red.add(where)
      }
    }
    return { orange: [...found.orange].sort(), red: [...found.red].sort() }
  }, DIALOG)
}

async function scrollToEnd(page: Page) {
  await page.locator(`${DIALOG} [data-scroll]`).evaluate((region) => {
    region.scrollTop = region.scrollHeight
  })
}

for (const colorScheme of ['light', 'dark'] as const) {
  // Exigence : EX-024-12
  test(`tout va bien, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'diagnostic-ok', colorScheme)
    await page.locator('[data-action="app-menu"]').click()
    await expect(page.locator('[data-menu="doctor"]')).toHaveText('Auto-diagnostic')
    await capture(page, `doctor-menu-${colorScheme}`)
    await page.locator('[data-menu="doctor"]').click()
    await done(page)
    const dialog = page.locator(DIALOG)
    await expect(dialog.locator('[data-summary]')).toHaveText('Tout va bien.')
    await expect(dialog.locator('[data-check-status]').first()).toHaveText('OK')
    expect(await fits(page)).toEqual([])
    // Black and white: nothing waits, nothing is broken.
    expect(await colored(page)).toEqual({ orange: [], red: [] })
    await capture(page, `doctor-ok-${colorScheme}`)
    await scrollToEnd(page)
    await expect(dialog.locator('[data-file]')).toContainText('doctor-20261123-100000.txt')
    await capture(page, `doctor-ok-fin-${colorScheme}`)
    expect(errors).toEqual([])
  })

  // Exigence : EX-024-12
  test(`avertissements, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'diagnostic-avertissements', colorScheme)
    await openDialog(page)
    await done(page)
    const dialog = page.locator(DIALOG)
    await expect(dialog.locator('[data-summary]')).toContainText('8 avertissements')
    await expect(dialog.locator('[data-check="notifications"] [data-fix]')).toContainText('« Notifications »')
    expect(await fits(page)).toEqual([])
    // Orange for what waits for an action, and no red.
    expect(await colored(page)).toEqual({
      orange: ['artifactory', 'clients', 'clock', 'installation', 'mcp', 'notifications', 'settings', 'summary', 'updateKey'],
      red: [],
    })
    await capture(page, `doctor-avertissements-${colorScheme}`)
    await scrollToEnd(page)
    await capture(page, `doctor-avertissements-fin-${colorScheme}`)
    expect(errors).toEqual([])
  })

  // Exigence : EX-024-12
  test(`échecs et marche à suivre, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'diagnostic-echecs', colorScheme)
    await openDialog(page)
    await done(page)
    const dialog = page.locator(DIALOG)
    await expect(dialog.locator('[data-summary]')).toContainText('9 échecs')
    await expect(dialog.locator('[data-check="webview"] [data-fix]')).toContainText('WebView2 Runtime')
    await expect(dialog.locator('[data-check="mcp"] [data-fix]')).toHaveCount(1)
    expect(await fits(page)).toEqual([])
    const { red } = await colored(page)
    expect(red).toEqual(
      ['app', 'artifactory', 'clients', 'installation', 'mcp', 'notifications', 'settings', 'summary', 'updateKey', 'webview'],
    )
    await capture(page, `doctor-echecs-${colorScheme}`)
    await dialog.locator('[data-check="settings"]').scrollIntoViewIfNeeded()
    await capture(page, `doctor-echecs-milieu-${colorScheme}`)
    await scrollToEnd(page)
    await capture(page, `doctor-echecs-fin-${colorScheme}`)
    expect(errors).toEqual([])
  })

  // Exigence : EX-024-12
  test(`rapport copié, ${colorScheme}`, async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write'])
    const errors = await open(page, 'diagnostic-avertissements', colorScheme)
    await openDialog(page)
    await done(page)
    const dialog = page.locator(DIALOG)
    await dialog.locator('[data-action="doctor-copy"]').click()
    await expect(dialog.locator('[data-copy]')).toHaveText('Rapport copié dans le presse-papiers.')
    const copied = await page.evaluate(() => navigator.clipboard.readText())
    expect(copied.startsWith('MCP Proxy — auto-diagnostic\n')).toBe(true)
    expect(copied).toContain('[avertissement] Notifications')
    expect(copied).toContain('Rapport enregistré dans %LOCALAPPDATA%\\MCP Proxy\\logs\\doctor-20261123-100000.txt')
    await scrollToEnd(page)
    await capture(page, `doctor-copie-${colorScheme}`)
    await dialog.locator('[data-action="doctor-close"]').click()
    await expect(dialog).toHaveCount(0)
    expect(errors).toEqual([])
  })

  // Exigence : EX-024-12
  test(`contrôle en cours, ${colorScheme}`, async ({ page }) => {
    const errors = await open(page, 'diagnostic-en-cours', colorScheme)
    await openDialog(page)
    const dialog = page.locator(DIALOG)
    await expect(dialog).toHaveAttribute('data-phase', 'running')
    await expect(dialog.locator('[data-running]')).toContainText('Contrôle en cours…')
    await expect(dialog.locator('[data-action="doctor-copy"]')).toBeDisabled()
    await expect(dialog.locator('[data-action="doctor-again"]')).toBeDisabled()
    expect(await fits(page)).toEqual([])
    expect(await colored(page)).toEqual({ orange: [], red: [] })
    await capture(page, `doctor-en-cours-${colorScheme}`)
    expect(errors).toEqual([])
  })
}
