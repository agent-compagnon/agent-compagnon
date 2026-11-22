import { type Page, expect, test } from '@playwright/test'

// The Journal tab on `vite preview`, with the simulated backend and the CSP of the application (spec 018).
const NOW = new Date('2026-11-22T14:00:00Z')
const CAPTURES = process.env.CAPTURES_DIR

/** Opens a scenario on the Journal tab; returns the errors of the page, CSP violations included. */
async function openJournal(page: Page, scenario: string, colorScheme: 'light' | 'dark' = 'light'): Promise<string[]> {
  const errors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', (error) => errors.push(error.message))
  await page.emulateMedia({ colorScheme })
  await page.clock.setFixedTime(NOW)
  await page.goto(`/index.html?scenario=${scenario}`)
  await page.getByRole('tab', { name: 'Journal' }).click()
  await expect(page.locator('[data-field="journal-count"]')).toBeVisible()
  return errors
}

async function capture(page: Page, name: string) {
  if (!CAPTURES) return
  await page.mouse.move(0, 0)
  await page.waitForTimeout(250)
  await page.screenshot({ path: `${CAPTURES}/${name}.png` })
}

/** Visible orange elements (hue 20°–80° with chroma): the journal has none, nothing in it waits for the user. */
async function strayOrange(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const canvas = document.createElement('canvas').getContext('2d')
    const found: string[] = []
    for (const element of document.querySelectorAll<HTMLElement>('body *')) {
      if (element.getClientRects().length === 0) continue
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
  })
}

/** Rendered rows, in order: each starts where the one above ends (no overlap, no gap). */
async function rowsAreStacked(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const rows = [...document.querySelectorAll<HTMLElement>('[data-scroller] [data-index]')]
      .map((e) => ({ index: Number(e.dataset.index), rect: e.getBoundingClientRect() }))
      .sort((a, b) => a.index - b.index)
    return rows.slice(1).flatMap((row, i) => {
      const above = rows.at(i)
      if (above === undefined || row.index !== above.index + 1 || Math.abs(row.rect.top - above.rect.bottom) <= 1)
        return []
      return [`row ${String(row.index)} starts at ${String(row.rect.top)}, above ends at ${String(above.rect.bottom)}`]
    })
  })
}

const topSeq = (page: Page) => page.locator('[data-entry]').first().getAttribute('data-entry')

for (const scheme of ['light', 'dark'] as const) {
  // Exigence : EX-018-06
  test(`Journal tab captures, ${scheme}`, async ({ page }) => {
    const errors = await openJournal(page, 'sessions', scheme)
    await expect(page.locator('[data-field="journal-count"]')).toHaveText('60 entrées')
    expect(await strayOrange(page), 'no orange in the journal').toEqual([])
    await capture(page, `journal-liste-${scheme}`)

    const error = page.locator('[data-entry][data-status="error"]').first()
    await error.locator('[data-action="toggle"]').click()
    await error.locator('[data-detail="response"]').scrollIntoViewIfNeeded()
    await expect(error.locator('[data-detail="request"]')).toBeVisible()
    expect(await rowsAreStacked(page)).toEqual([])
    await capture(page, `journal-ligne-depliee-${scheme}`)
    await error.locator('[data-action="toggle"]').click()

    await page.locator('[data-status-filter="problems"]').click()
    await expect(page.locator('[data-entry][data-expandable="false"][data-kind="call"]:not([data-status])')).toHaveCount(0)
    await capture(page, `journal-problemes-${scheme}`)
    await page.locator('[data-status-filter="all"]').click()

    await page.locator('[data-field="journal-search"]').fill('kibána réussi')
    await expect(page.locator('[data-entry] [data-highlight]').first()).toBeVisible()
    await capture(page, `journal-recherche-${scheme}`)
    await page.locator('[data-field="journal-search"]').fill('')

    await page.locator('[data-action="export"]').click()
    await expect(page.locator('[data-banner="export-saved"]')).toContainText(
      'C:\\Users\\exemple\\Downloads\\mcp-proxy-diagnostic-20261122-1400.zip',
    )
    await capture(page, `journal-export-${scheme}`)

    const failed = await openJournal(page, 'export-echec', scheme)
    await page.locator('[data-action="export"]').click()
    await expect(page.locator('[data-banner="export-failed"]')).toContainText('Export de diagnostic impossible : accès refusé.')
    await capture(page, `journal-export-echec-${scheme}`)

    const empty = await openJournal(page, 'journal-vide', scheme)
    await expect(page.locator('[data-empty="journal"]')).toBeVisible()
    await capture(page, `journal-vide-${scheme}`)

    const live = await openJournal(page, 'journal-direct', scheme)
    const first = Number(await topSeq(page))
    await expect.poll(async () => Number(await topSeq(page)), { timeout: 10_000 }).toBeGreaterThan(first)
    await page.locator('[data-action="live"]').click()
    const paused = await topSeq(page)
    await expect
      .poll(async () => Number(await page.locator('[data-field="pending"]').getAttribute('data-pending')), {
        timeout: 10_000,
      })
      .toBeGreaterThanOrEqual(2)
    expect(await topSeq(page), 'in pause the list does not move').toBe(paused)
    await capture(page, `journal-pause-${scheme}`)
    await page.locator('[data-action="show-pending"]').click()
    expect(Number(await topSeq(page))).toBeGreaterThan(Number(paused))

    expect([...errors, ...failed, ...empty, ...live], 'no CSP violation nor error').toEqual([])
  })
}

// Exigence : EX-018-06
// Exigence : EX-018-05
test('several thousand entries scroll smoothly, unfolded rows included', async ({ page }) => {
  const errors = await openJournal(page, 'journal-volumineux')
  await expect(page.locator('[data-field="journal-count"]')).toHaveText('5 000 entrées')
  await expect(page.locator('[data-field="limit"]')).toBeVisible()
  // Only the rows in view are in the DOM.
  expect(await page.locator('[data-entry]').count()).toBeLessThan(60)

  const scroller = page.locator('[data-scroller]')
  await page.locator('[data-entry][data-expandable="true"] [data-action="toggle"]').first().click()
  await scroller.evaluate((e) => {
    e.scrollTop = e.scrollHeight / 2
  })
  await page.waitForTimeout(100)
  const middle = page.locator('[data-entry][data-expandable="true"]').nth(2)
  await middle.locator('[data-action="toggle"]').click()
  await expect(middle.locator('[data-details]')).toBeVisible()
  await middle.locator('[data-detail="response"]').scrollIntoViewIfNeeded()
  await page.waitForTimeout(100)
  expect(await rowsAreStacked(page)).toEqual([])
  if (CAPTURES) await page.screenshot({ path: `${CAPTURES}/journal-volumineux-milieu.png` })
  await scroller.evaluate((e) => {
    e.scrollTop += 400
  })
  await page.waitForTimeout(100)
  expect(await rowsAreStacked(page)).toEqual([])

  // The oldest entry kept is reached at the bottom.
  await scroller.evaluate((e) => {
    e.scrollTop = e.scrollHeight
  })
  await expect(page.locator('[data-entry="1"]')).toBeVisible()
  expect(await rowsAreStacked(page)).toEqual([])
  expect(errors).toEqual([])
})

// Exigence : EX-018-02
test('filters combine: environment, status and search', async ({ page }) => {
  await openJournal(page, 'sessions')
  await page.locator('[data-field="journal-environment"]').click()
  await page.locator('[data-choice="prod"]').click()
  await page.locator('[data-status-filter="policies"]').click()
  const envs = await page.locator('[data-entry] [data-cell="environment"]').allTextContents()
  expect(envs.length).toBeGreaterThan(0)
  expect(new Set(envs)).toEqual(new Set(['PROD']))
  await page.locator('[data-field="journal-search"]').fill('cancel_pipeline')
  await expect(page.locator('[data-entry]').first()).toBeVisible()
  for (const tool of await page.locator('[data-entry] [data-cell="tool"]').allTextContents())
    expect(tool).toBe('prod_gitlab_cancel_pipeline')
  await page.locator('[data-field="journal-search"]').fill('aucun-outil-de-ce-nom')
  await expect(page.locator('[data-empty="filtered"]')).toBeVisible()
  await page.locator('[data-action="clear-filters"]').click()
  await expect(page.locator('[data-field="journal-count"]')).toHaveText('60 entrées')
})

// Exigence : EX-018-03
test('only the rows the core kept unfold, by keyboard too', async ({ page }) => {
  await openJournal(page, 'sessions')
  const normal = page.locator('[data-entry][data-status="ok"][data-expandable="false"]').first()
  await normal.click()
  await expect(normal.locator('[data-details]')).toHaveCount(0)
  await expect(normal.locator('[data-action="toggle"]')).toHaveCount(0)
  const blocked = page.locator('[data-entry][data-status="blocked"]').first()
  await blocked.locator('[data-action="toggle"]').focus()
  await page.keyboard.press('Enter')
  await expect(blocked.locator('[data-detail="request"]')).toContainText('"body": "[SECRET MASQUÉ]"')
})
