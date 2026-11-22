import { type Page, expect, test } from '@playwright/test'

// The MCP tab on `vite preview`, with the simulated backend and the CSP of the application (spec 017).
const NOW = new Date('2026-11-22T14:00:00Z')
const CAPTURES = process.env.CAPTURES_DIR

/** Opens a scenario on the MCP tab; returns the errors of the page, CSP violations included. */
async function openMcp(page: Page, scenario: string, colorScheme: 'light' | 'dark' = 'light'): Promise<string[]> {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', (error) => errors.push(error.message))
  await page.emulateMedia({ colorScheme })
  await page.clock.setFixedTime(NOW)
  await page.goto(`/index.html?scenario=${scenario}`)
  await page.getByRole('tab', { name: 'MCP' }).click()
  await expect(page.locator('[data-counter]')).toBeVisible()
  return errors
}

/** The whole window, the scrolled content included: the viewport grows to its height for the capture. */
async function capture(page: Page, name: string) {
  if (!CAPTURES) return
  await page.mouse.move(0, 0)
  await page.waitForTimeout(250)
  const height = await page.evaluate(() => {
    const main = document.querySelector('main')
    const header = document.querySelector('header')
    return (main?.scrollHeight ?? 0) + (header?.offsetHeight ?? 0)
  })
  const viewport = page.viewportSize() ?? { width: 960, height: 680 }
  await page.setViewportSize({ width: viewport.width, height: Math.max(viewport.height, height) })
  await page.waitForTimeout(100)
  await page.screenshot({ path: `${CAPTURES}/${name}.png` })
  await page.setViewportSize(viewport)
}

/** Visible orange elements outside of `allowed` (same rule as the Connections tab: hue 20°–80° with chroma). */
async function strayOrange(page: Page, allowed: string): Promise<string[]> {
  return page.evaluate((allowedSelector) => {
    const canvas = document.createElement('canvas').getContext('2d')
    const found: string[] = []
    for (const element of document.querySelectorAll<HTMLElement>('body *')) {
      if ((allowedSelector && element.closest(allowedSelector)) || element.getClientRects().length === 0) continue
      const style = getComputedStyle(element)
      for (const color of [style.color, style.backgroundColor, style.borderTopColor]) {
        if (!canvas || color === '' || color === 'transparent') continue
        canvas.clearRect(0, 0, 1, 1)
        canvas.fillStyle = '#000'
        canvas.fillStyle = color
        canvas.fillRect(0, 0, 1, 1)
        const [r = 0, g = 0, b = 0, a = 0] = canvas.getImageData(0, 0, 1, 1).data
        const max = Math.max(r, g, b)
        const min = Math.min(r, g, b)
        if (max === min || a / 255 < 0.05 || (max - min) / 255 <= 0.15) continue
        let h = max === r ? ((g - b) / (max - min)) % 6 : max === g ? (b - r) / (max - min) + 2 : (r - g) / (max - min) + 4
        h *= 60
        if (h < 0) h += 360
        if (h >= 20 && h <= 80) found.push(`${element.tagName.toLowerCase()}: ${color}`)
      }
    }
    return found
  }, allowed)
}

for (const scheme of ['light', 'dark'] as const) {
  // Exigence : EX-017-07
  test(`MCP tab captures, ${scheme}`, async ({ page }) => {
    const errors = await openMcp(page, 'sessions', scheme)
    await expect(page.locator('[data-row]')).toHaveCount(24)
    expect(await strayOrange(page, ''), 'below the threshold: no orange').toEqual([])
    await capture(page, `mcp-liste-${scheme}`)

    await page.locator('[data-field="search"]').fill('kibána index')
    await expect(page.locator('[data-tools] [data-highlight]').first()).toBeVisible()
    await capture(page, `mcp-recherche-${scheme}`)
    await page.locator('[data-field="search"]').fill('')

    await page.locator('[data-field="environment-filter"]').click()
    await page.locator('[data-choice="prod"]').click()
    await expect(page.locator('[data-row]')).toHaveCount(5)
    await capture(page, `mcp-filtre-prod-non-expose-${scheme}`)

    await openMcp(page, 'mcp-mis-a-jour', scheme)
    await capture(page, `mcp-bandeau-${scheme}`)
    await page.locator('[data-banner="mcp-update"][data-update="rec/kibana-logs"]').getByRole('button', { name: /^Voir/ }).click()
    await expect(page.locator('[data-tools="rec/kibana-logs"] [data-mark="added"]')).toHaveCount(2)
    await capture(page, `mcp-bandeau-voir-${scheme}`)

    await openMcp(page, 'seuil', scheme)
    await expect(page.locator('[data-counter]')).toHaveAttribute('data-over', 'true')
    expect(await strayOrange(page, '[data-counter]'), 'orange on the counter only').toEqual([])
    await page.locator('[data-counter]').hover()
    await expect(page.getByRole('tooltip')).toContainText('plafonnent à 128 outils')
    await page.waitForTimeout(300)
    if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/mcp-seuil-${scheme}.png` })

    await openMcp(page, 'rien-expose', scheme)
    await expect(page.locator('[data-empty="exposed"]')).toBeVisible()
    await page.locator('[data-field="search"]').fill('grafana')
    await expect(page.locator('[data-empty="rows"]')).toContainText('Aucun MCP ne correspond à « grafana ».')
    await capture(page, `mcp-vide-${scheme}`)

    await openMcp(page, 'volumineux', scheme)
    await expect(page.locator('[data-row]')).toHaveCount(40)
    await capture(page, `mcp-volumineux-${scheme}`)
    expect(errors, 'no CSP violation nor error').toEqual([])
  })
}

// Exigence : EX-017-02
test('the search unfolds the rows whose tools match, accents and case aside', async ({ page }) => {
  await openMcp(page, 'sessions')
  await page.locator('[data-field="search"]').fill('PROD DÉLETE')
  const rows = page.locator('[data-row]')
  await expect(rows.first()).toBeVisible()
  for (const id of await rows.evaluateAll((els) => els.map((e) => e.getAttribute('data-row')))) {
    expect(id, "« prod » is also in « PPROD »").toMatch(/^p?prod\//)
  }
  await expect(page.locator('[data-row="prod/kibana-logs"]')).toHaveAttribute('data-open', 'true')
  const tools = page.locator('[data-tools="prod/kibana-logs"] [data-tool]')
  await expect(tools).toHaveCount(1)
  await expect(tools.first().locator('[data-highlight]').first()).toHaveText('delete')
  // A description written by a server is text: no markup of its own reaches the page.
  await expect(page.locator('[data-tools] mark:not([data-highlight])')).toHaveCount(0)
})

// Exigence : EX-017-04
test('pastilles and switches send their commands; a disabled switch says why', async ({ page }) => {
  await openMcp(page, 'sessions')
  const qua = page.locator('[data-environment="qua"]')
  await expect(qua).toHaveAttribute('aria-pressed', 'false')
  await qua.click()
  await expect(qua).toHaveAttribute('aria-pressed', 'true')
  await expect(page.locator('[data-row="qua/kibana-logs"] [data-action="expose"]')).toBeEnabled()

  // A sensitive environment asks first.
  await page.locator('[data-environment="prod"]').click()
  const dialog = page.getByRole('alertdialog')
  await expect(dialog).toContainText('Exposer PROD aux clients LLM ?')
  await dialog.getByRole('button', { name: 'Annuler' }).click()
  await expect(page.locator('[data-environment="prod"]')).toHaveAttribute('aria-pressed', 'false')
  const prodSwitch = page.locator('[data-row="prod/kibana-logs"] [data-action="expose"]')
  await expect(prodSwitch).toBeDisabled()
  await expect(page.locator('[data-row="prod/kibana-logs"] [data-field="why"]')).toHaveText(
    "PROD n'est pas exposé : exposez-le d'abord avec sa pastille.",
  )
  await page.locator('[data-environment="prod"]').click()
  await page.getByRole('alertdialog').getByRole('button', { name: 'Exposer' }).click()
  await expect(prodSwitch).toBeEnabled()

  // Cutting an MCP cuts it everywhere.
  await page.locator('[data-row="dev/kibana-logs"] [data-action="expose"]').click()
  await expect(page.locator('[data-row="prod/kibana-logs"] [data-state="disabled"]')).toBeVisible()
  await expect(page.locator('[data-row="dev/gitlab-ops"] [data-action="expose"]')).toHaveAttribute('data-state', 'checked')
})

// Exigence : EX-017-05
test('« Voir » filters on the updated MCP, « J\'ai vu » acknowledges it', async ({ page }) => {
  await openMcp(page, 'mcp-mis-a-jour')
  const banner = page.locator('[data-banner="mcp-update"][data-update="rec/kibana-logs"]')
  await expect(banner).toContainText('Kibana Logs (REC) a été mis à jour : 2 nouveaux outils et 1 outil modifié.')
  await banner.getByRole('button', { name: /^Voir/ }).click()
  await expect(page.locator('[data-row]')).toHaveCount(1)
  await expect(page.locator('[data-field="focus"]')).toContainText('Filtré sur Kibana Logs (REC)')
  const fresh = page.locator('[data-tools="rec/kibana-logs"] [data-tool="export_logs"]')
  await expect(fresh.locator('[data-mark="added"]')).toBeVisible()
  await expect(fresh.locator('[data-mark="unknown"]')).toBeVisible()
  await expect(fresh.locator('[data-field="tool-policy"]')).toHaveText('confirmation par notification')
  await banner.getByRole('button', { name: /^J'ai vu/ }).click()
  await expect(banner).toHaveCount(0)
  await page.locator('[data-action="clear-focus"]').click()
  await expect(page.locator('[data-row]')).toHaveCount(24)
})

// Exigence : EX-017-03
test('the environment filter shows a hidden environment with its count', async ({ page }) => {
  await openMcp(page, 'sessions')
  await page.locator('[data-field="environment-filter"]').click()
  await expect(page.locator('[data-choice="pra"]')).toContainText('2')
  await page.locator('[data-choice="pra"]').click()
  await expect(page.locator('[data-row]')).toHaveCount(2)
  await expect(page.locator('[data-environment="pra"]')).toHaveAttribute('aria-pressed', 'false')
})
