import { expect, test, type Page } from '@playwright/test'
import { BRANCH_COUNT, CATALOG_COUNT, RARE_COUNT, RARE_WORD } from './fixtures'
import { CARD, measure, openApp, scrollerTop, scrollTo, selectedInView, SELECTED, settle, waitForCards, type Measure, type Scope } from './helpers'

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')

// One screenshot per case, also for a failing one.
test.afterEach(async ({ page }, info) => {
  await page.screenshot({ path: `e2e/screenshots/${slug(info.title)}.png` }).catch(() => undefined)
})

const COUNT: Record<Scope, number> = { catalog: CATALOG_COUNT, branch: BRANCH_COUNT }

/** No uncovered stretch taller than a few pixels inside the visible grid area. */
function expectCovered(m: Measure, where: string) {
  expect(m.cards, `${where}: cards rendered`).toBeGreaterThan(0)
  expect(m.gap, `${where}: blank stretch in the visible grid area (${JSON.stringify(m)})`).toBeLessThanOrEqual(3)
}

async function bound(page: Page, where: string) {
  const m = await measure(page)
  // Rows that touch the viewport (a partly visible row counts), plus the grid's own overscan above and below.
  const pitch = m.rowHeight + m.rowGap
  const visibleRows = Math.ceil((m.viewTop + m.viewHeight) / pitch) - Math.floor(m.viewTop / pitch)
  const limit = (visibleRows + 6) * m.columns
  expect(m.cards, `${where}: ${m.cards} cards, limit ${limit} (${visibleRows} visible rows x ${m.columns} columns)`).toBeLessThanOrEqual(limit)
  expect(m.cards, `${where}: far below the full list`).toBeLessThan(200)
  expectCovered(m, where)
}

async function checkBound(page: Page) {
  await bound(page, 'top')
  const m = await measure(page)
  await scrollTo(page, Math.round((m.scrollHeight - m.viewHeight) / 2))
  await settle(page)
  await bound(page, 'middle')
  await scrollTo(page, 'end')
  await settle(page)
  await bound(page, 'end')
}

async function checkEnd(page: Page, total: number) {
  await scrollTo(page, 'end')
  await settle(page)
  const m = await measure(page)
  expectCovered(m, 'end')
  const base = 14
  expect(m.padBottom, `bottom padding at the end (${JSON.stringify(m)})`).toBeLessThanOrEqual(base + 2)
  // The last card sits at the bottom of the scroll height, so the last row is rendered.
  expect(m.scrollTop + m.viewHeight, 'scrolled to the end').toBeGreaterThanOrEqual(m.scrollHeight - 2)
  expect(m.gridBottom - m.padBottom - m.lastCardBottom, `last card to the grid's bottom edge (${JSON.stringify(m)})`).toBeLessThanOrEqual(2)
  const partial = total % m.columns || m.columns
  expect(m.lastRowCards, `cards in the final row (${total} items, ${m.columns} columns)`).toBe(partial)
}

async function selectFirstCard(page: Page) {
  await page.locator(CARD).first().click()
  await expect(page.locator(SELECTED)).toHaveCount(1)
  await settle(page)
}

async function checkArrows(page: Page) {
  await selectFirstCard(page)
  const check = async (where: string) => {
    await settle(page)
    const m = await measure(page)
    const top = await scrollerTop(page)
    expect(m.selected, `${where}: a card is marked selected (aria-pressed)`).toBeTruthy()
    expect(selectedInView(m, top), `${where}: selected card inside the viewport (${JSON.stringify(m.selected)}, view ${top}..${top + m.viewHeight})`).toBe(true)
    expect(m.selected!.underToolbar, `${where}: selected card hidden under the sticky toolbar`).toBe(false)
    expectCovered(m, where)
  }
  for (let i = 1; i <= 30; i++) {
    await page.keyboard.press('ArrowDown')
    if (i === 10 || i === 20) await check(`after ${i} ArrowDown`)
  }
  await check('after 30 ArrowDown')
  for (let i = 1; i <= 15; i++) {
    await page.keyboard.press('ArrowRight')
    if (i === 5) await check(`after ${i} ArrowRight`)
  }
  await check('after 15 ArrowRight')
}

test('1. cached catalog shows before the scan finishes', async ({ page }) => {
  await openApp(page, 'catalog', { scanDelayMs: 5000 })
  const started = Date.now()
  await page.locator(CARD).first().waitFor({ timeout: 1500 })
  const elapsed = Date.now() - started
  const calls = await page.evaluate(() => window.__e2e.calls)
  expect(calls, 'scan was asked for').toContain('catalog_scan')
  expect(elapsed, 'cards on screen well before the 5 s scan ends').toBeLessThan(1500)
})

test('1b. cards show without waiting for the repo list or the scan', async ({ page }) => {
  await openApp(page, 'catalog', { reposDelayMs: 4000, scanDelayMs: 5000 })
  const started = Date.now()
  await page.locator(CARD).first().waitFor({ timeout: 1500 })
  expect(Date.now() - started, 'cards before the repo list (4 s) and the scan (5 s)').toBeLessThan(1500)
})

test('1c. opening makes exactly one catalog read', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  await page.waitForTimeout(2000)
  const reads = await page.evaluate(() => window.__e2e.calls.filter((c) => c === 'catalog_list').length)
  expect(reads, 'catalog_list calls in the first seconds after opening').toBe(1)
})

test('1d. a slow read lands while status events keep coming, one read at a time', async ({ page }) => {
  await openApp(page, 'catalog', { listDelayMs: 2000 })
  await page.evaluate(() => {
    let done = 0
    setInterval(() => void window.__e2e.emitStatus({ state: 'running', done: ++done, total: 100_000 }), 300)
  })
  await page.locator(CARD).first().waitFor({ timeout: 4000 })
  await page.waitForTimeout(1500)
  const { max, reads } = await page.evaluate(() => ({ max: window.__e2e.listMaxInFlight, reads: window.__e2e.calls.filter((c) => c === 'catalog_list').length }))
  expect(reads, 'at least one read').toBeGreaterThanOrEqual(1)
  expect(max, 'catalog_list calls running at once').toBe(1)
})

test('1e. a snapshot shows cards while the real read is slow, then the real listing replaces it', async ({ page }) => {
  await openApp(page, 'catalog', { snapshotCount: 40, listDelayMs: 3000 })
  const started = Date.now()
  await page.locator(CARD).first().waitFor({ timeout: 1000 })
  expect(Date.now() - started, 'cards from the snapshot well before the 3 s read ends').toBeLessThan(1000)
  const early = await measure(page)
  expect(early.scrollHeight, 'only the snapshot is laid out').toBeLessThan(5000)
  // The real listing (5003 assets) lands and replaces it.
  await expect.poll(async () => (await measure(page)).scrollHeight, { timeout: 10_000 }).toBeGreaterThan(20_000)
  const calls = await page.evaluate(() => window.__e2e.calls)
  expect(calls, 'the snapshot was asked for').toContain('catalog_snapshot')
})

test('2. no inline indexing bar; tasks popover pauses and resumes', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  await page.evaluate(() => window.__e2e.emitStatus({ state: 'running', done: 3, total: 20 }))
  const trigger = page.locator('.popover-trigger:has(.tasks-trigger)')
  await trigger.waitFor()
  await expect(page.locator('.content .catalog-status')).toHaveCount(0)
  await expect(page.locator('.catalog-status')).toHaveCount(0)
  await trigger.click()
  const pause = page.locator('.tasks-popover .task-row button', { hasText: 'Pause' })
  await expect(pause).toBeVisible()
  await pause.click()
  const resume = page.locator('.tasks-popover .task-row button', { hasText: 'Resume' })
  await expect(resume).toBeVisible()
  expect(await page.evaluate(() => window.__e2e.calls.filter((c) => c === 'catalog_pause').length)).toBe(1)
  await resume.click()
  await expect(page.locator('.tasks-popover .task-row button', { hasText: 'Pause' })).toBeVisible()
  expect(await page.evaluate(() => window.__e2e.calls.filter((c) => c === 'catalog_resume').length)).toBe(1)
})

test('3. error status shows the inline error line with Retry', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  await page.evaluate(() => window.__e2e.emitStatus({ state: 'error', done: 1, total: 20, error: { code: 'rateLimited', message: 'rate limited', resetAt: Date.now() + 3_600_000 } }))
  const line = page.locator('.content .catalog-status')
  await expect(line).toHaveCount(1)
  await expect(line).toContainText('request limit reached')
  const retry = line.locator('button')
  await expect(retry).toHaveCount(1)
  await retry.click()
  expect(await page.evaluate(() => window.__e2e.calls.filter((c) => c === 'catalog_resume').length)).toBe(1)
})

test('4. virtual grid keeps the rendered cards bounded', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  await checkBound(page)
})

test('5. scrolling to the end renders the last asset and a partial final row', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  await checkEnd(page, COUNT.catalog)
})

test('6. fast scrolling never leaves a blank viewport', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  // Status and progress events keep arriving while the list is scrolled.
  await page.evaluate(() => {
    let done = 0
    setInterval(() => void window.__e2e.emitStatus({ state: 'running', done: ++done, total: 100_000 }), 40)
  })
  const first = await measure(page)
  const travel = first.scrollHeight - first.viewHeight
  let seed = 12345
  const next = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296)
  for (let i = 0; i < 20; i++) {
    const [a, b] = [Math.floor(next() * travel), Math.floor(next() * travel)]
    // Two jumps in a row, the second one lands.
    await scrollTo(page, a)
    await scrollTo(page, b)
    await settle(page)
    const m = await measure(page)
    expectCovered(m, `jump ${i + 1} to ${b}`)
  }
})

test('7. resize and cell size keep cards in the viewport and the selection visible', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  // Open the details panel first (it narrows the grid), so the steps below are the only layout changes.
  await selectFirstCard(page)
  const first = await measure(page)
  await scrollTo(page, Math.round((first.scrollHeight - first.viewHeight) * 0.6))
  await settle(page)
  const card = page.locator(CARD).nth(Math.floor((await page.locator(CARD).count()) / 2))
  await card.scrollIntoViewIfNeeded()
  await card.click()
  await expect(page.locator(SELECTED)).toHaveCount(1)
  const steps: Array<[string, () => Promise<void>]> = [
    ['narrow window', () => page.setViewportSize({ width: 1000, height: 600 })],
    ['tall window', () => page.setViewportSize({ width: 1500, height: 1100 })],
    ['large cells', async () => void (await page.locator('.size-toggle button').nth(1).click())],
    ['small cells', async () => void (await page.locator('.size-toggle button').nth(0).click())],
    ['back to 1400x900', () => page.setViewportSize({ width: 1400, height: 900 })]
  ]
  for (const [name, run] of steps) {
    await run()
    await settle(page, 4)
    const m = await measure(page)
    const top = await scrollerTop(page)
    expectCovered(m, name)
    expect(m.selected, `${name}: the selected card is rendered (scrollTop ${m.scrollTop} of ${m.scrollHeight}, ${m.columns} columns)`).toBeTruthy()
    expect(selectedInView(m, top), `${name}: selected card in the viewport (${JSON.stringify(m.selected)}, view ${top}..${top + m.viewHeight})`).toBe(true)
  }
})

// Extra: the same jump seen from the other side. Selecting a card opens the details panel, which narrows the grid.
test('7b. opening the details panel deep in the list keeps the place', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  const first = await measure(page)
  const target = Math.round((first.scrollHeight - first.viewHeight) * 0.6)
  await scrollTo(page, target)
  await settle(page)
  const before = await measure(page)
  const card = page.locator(CARD).nth(Math.floor((await page.locator(CARD).count()) / 2))
  const id = await card.getAttribute('data-asset-id')
  await card.click()
  await settle(page, 30)
  const after = await measure(page)
  // Fewer columns make the list taller, so the same place is further down; it must not fall back to the top.
  expect(after.scrollTop, `scrollTop before ${before.scrollTop} (${before.columns} columns), after ${after.scrollTop} (${after.columns} columns, height ${after.scrollHeight})`).toBeGreaterThanOrEqual(before.scrollTop * 0.5)
  await expect(page.locator(`${CARD}[data-asset-id="${id}"]`)).toHaveCount(1)
})

test('8. arrow keys keep the selected card in view', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  await checkArrows(page)
})

test('9. a short result list leaves no large padding', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  await page.locator('#q').fill(RARE_WORD)
  await expect(page.locator(CARD)).toHaveCount(RARE_COUNT, { timeout: 10_000 })
  await settle(page, 4)
  const m = await measure(page)
  const rows = Math.ceil(RARE_COUNT / m.columns)
  const content = rows * m.rowHeight + (rows - 1) * m.rowGap
  const grid = m.gridBottom - m.gridTop
  // Own padding is 14 px above and below; nothing else may be left over.
  expect(grid - content, `grid ${grid}px vs rows ${content}px (${JSON.stringify(m)})`).toBeLessThanOrEqual(14 * 2 + 4)
  expect(m.scrollHeight, 'nothing to scroll for a short list').toBeLessThanOrEqual(m.viewHeight + 1)
})

test('10. no card clips its content', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  const seen = { notes: 0, facts: 0, cards: 0 }
  const clipped: string[] = []
  const first = await measure(page)
  const travel = first.scrollHeight - first.viewHeight
  for (const at of [0, 0.25, 0.5, 0.75, 1]) {
    await scrollTo(page, Math.round(travel * at))
    await settle(page)
    const found = await page.evaluate((selector) => {
      const out = { notes: 0, facts: 0, cards: 0, clipped: [] as string[] }
      for (const card of document.querySelectorAll<HTMLElement>(selector)) {
        out.cards++
        if (card.querySelector('.asset-note')) out.notes++
        if (card.querySelectorAll('.asset-fact > *').length > 1) out.facts++
        if (card.scrollHeight > card.offsetHeight + 1) out.clipped.push(`${card.dataset.assetId}: scrollHeight ${card.scrollHeight} > offsetHeight ${card.offsetHeight}`)
      }
      return out
    }, CARD)
    seen.notes += found.notes
    seen.facts += found.facts
    seen.cards += found.cards
    clipped.push(...found.clipped)
  }
  expect(seen.notes, 'cards with a note were checked').toBeGreaterThan(0)
  expect(seen.facts, 'cards with a fact line were checked').toBeGreaterThan(0)
  expect(clipped, `clipped cards of ${seen.cards} checked`).toEqual([])
})

test.describe('11. branch scope', () => {
  test('bound (case 4)', async ({ page }) => {
    await openApp(page, 'branch')
    await waitForCards(page)
    await checkBound(page)
  })
  test('scroll to the end (case 5)', async ({ page }) => {
    await openApp(page, 'branch')
    await waitForCards(page)
    await checkEnd(page, COUNT.branch)
  })
  test('arrow keys (case 8)', async ({ page }) => {
    await openApp(page, 'branch')
    await waitForCards(page)
    await checkArrows(page)
  })
})
