import { expect, test, type Page } from '@playwright/test'
import { BRANCH_COUNT, branchAssets, CATALOG_COUNT, catalogAssets, RARE_COUNT, RARE_WORD } from './fixtures'
import { THUMB_CACHE_CAP } from '../ui/src/thumbCache'
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
  // Placeholder cards cover complete rows only: none for the empty cells after the last card.
  const pitch = m.rowHeight + m.rowGap
  const complete = Math.floor(RARE_COUNT / m.columns)
  const layer = await page.evaluate(() => parseFloat(getComputedStyle(document.querySelector('.content .grid')!, '::before').height))
  expect(layer, `placeholder layer for ${RARE_COUNT} results in ${m.columns} columns`).toBeLessThanOrEqual(Math.max(0, complete * pitch - m.rowGap) + 1)
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

test('10b. thumbnails render only for cards on screen, and scrolling renders the newly visible ones', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  // Each render attempt reads the file once (the test backend has none, so every attempt fails at once).
  const reads = () => page.evaluate(() => window.__e2e.calls.filter((c) => c === 'assets_read_file').length)
  await page.waitForTimeout(1500)
  const idle = await reads()
  const rendered = (await measure(page)).cards
  expect(idle, `file reads with the grid idle: ${idle}, cards in the DOM: ${rendered}`).toBeLessThanOrEqual(rendered)
  await page.waitForTimeout(1500)
  expect(await reads(), 'no further reads while idle').toBe(idle)
  await scrollTo(page, 'end')
  await settle(page)
  await expect.poll(reads, { message: 'newly visible cards render' }).toBeGreaterThan(idle)
  expect(await reads(), 'reads stay far below the list size').toBeLessThan(400)
})

test('10c. finishing thumbnails never blocks the main thread on a 375,000 asset catalog', async ({ page }) => {
  test.setTimeout(240_000)
  // Files read as a small PNG, so the grid's image thumbnails render for real and finish while it scrolls.
  await openApp(page, 'catalog', { catalogCount: 375_000, readableFiles: true })
  await waitForCards(page)
  // Let the opening work (indexing, lookups) finish, so only what thumbnails cost is measured.
  await page.waitForTimeout(3000)
  await page.evaluate(() => {
    const w = window as unknown as { __blocks: number[]; __worstFrame: number }
    w.__blocks = []
    w.__worstFrame = 0
    new PerformanceObserver((list) => list.getEntries().forEach((e) => w.__blocks.push(e.duration))).observe({ type: 'longtask' })
    // Longest wait for an animation frame: a blocked main thread shows up here even before it counts as a long task.
    let last = performance.now()
    const tick = () => {
      const now = performance.now()
      w.__worstFrame = Math.max(w.__worstFrame, now - last)
      last = now
      requestAnimationFrame(tick)
    }
    requestAnimationFrame(tick)
  })
  const saves = () => page.evaluate(() => window.__e2e.calls.filter((c) => c === 'thumbs_save').length)
  const before = await saves()
  // Scrolling brings new cards, so thumbnails keep finishing.
  const m = await measure(page)
  const step = m.viewHeight / 2
  for (let at = 0; at < 60 && (await saves()) - before < 12; at++) {
    await scrollTo(page, Math.round((at + 1) * step))
    await page.waitForTimeout(400)
  }
  await page.waitForTimeout(1500)
  const finished = (await saves()) - before
  const { blocks, worstFrame } = await page.evaluate(() => {
    const w = window as unknown as { __blocks: number[]; __worstFrame: number }
    return { blocks: w.__blocks, worstFrame: w.__worstFrame }
  })
  console.log(`${finished} thumbnails finished at 375k: long tasks ${JSON.stringify(blocks.map(Math.round))}, worst frame ${Math.round(worstFrame)} ms`)
  expect(finished, 'thumbnails finished while scrolling').toBeGreaterThanOrEqual(8)
  expect(Math.max(0, ...blocks), `longest main-thread block while ${finished} thumbnails finished: ${JSON.stringify(blocks)}`).toBeLessThan(200)
  expect(worstFrame, 'worst wait for an animation frame').toBeLessThan(250)
})

test('10d. a background fact scan never blocks the main thread on a 375,000 asset catalog', async ({ page }) => {
  test.setTimeout(240_000)
  // Two scans (effects, then UI prefabs) find 6,000 prefab SHAs each over about six seconds; the UI ones start half way into the effects ones.
  const scan = { perFlush: 200, intervalMs: 200, flushes: 30 }
  const found = scan.perFlush * scan.flushes
  const prefabs = catalogAssets(375_000).filter((a) => a.type === 'prefab').length
  await openApp(page, 'catalog', { catalogCount: 375_000, factScan: scan })
  await waitForCards(page)
  // Let the opening work (indexing, lookups) finish, so only what the scan's findings cost is measured.
  await page.waitForTimeout(2500)
  await page.evaluate(() => {
    const w = window as unknown as { __blocks: number[] }
    w.__blocks = []
    new PerformanceObserver((list) => list.getEntries().forEach((e) => w.__blocks.push(e.duration))).observe({ type: 'longtask' })
  })
  const scans = () => page.evaluate(() => window.__e2e.calls.filter((c) => c === 'assets_known_assets').length)
  await expect.poll(scans, { timeout: 30_000, message: 'both fact scans started' }).toBe(2)
  // Both are over once the sidebar shows the last findings.
  const count = (label: string) => page.locator('.tab-sub', { hasText: label }).locator('.num')
  await expect(count('Particle System')).toHaveText(String(found), { timeout: 60_000 })
  await expect(count('UI')).toHaveText(String(found), { timeout: 60_000 })
  // Prefabs with either tag are not Other: the two sets overlap by half.
  await expect(count('Other')).toHaveText(String(prefabs - found * 1.5), { timeout: 60_000 })
  const blocks = await page.evaluate(() => (window as unknown as { __blocks: number[] }).__blocks)
  console.log(`fact scan at 375k: long tasks ${JSON.stringify(blocks.map(Math.round))}, total ${Math.round(blocks.reduce((a, b) => a + b, 0))} ms`)
  expect(Math.max(0, ...blocks), `longest main-thread block while the scans reported: ${JSON.stringify(blocks)}`).toBeLessThan(100)
})

test('10e. saved thumbnails are read for the cards on screen only and held in a bounded cache', async ({ page }) => {
  test.setTimeout(240_000)
  const assets = catalogAssets(375_000)
  // Half of the blobs have a saved thumbnail.
  await openApp(page, 'catalog', { catalogCount: 375_000, cachedThumbs: 0.5 })
  await waitForCards(page)
  const m = await measure(page)
  // Jumps far down the list, in an order that never repeats a place: far more saved thumbnails are read than the cache holds.
  for (let k = 1; k <= 100; k++) {
    await scrollTo(page, Math.round((((k * 7919) % 1000) / 1000) * (m.scrollHeight - m.viewHeight)))
    await page.waitForTimeout(120)
  }
  await settle(page)
  await page.waitForTimeout(800)

  const e2e = await page.evaluate(() => ({ sizes: window.__e2e.thumbGetSizes, answered: window.__e2e.thumbAnswered, held: window.__e2eThumbCount?.() ?? -1 }))
  console.log(`${e2e.sizes.length} thumbs_get calls, largest ${Math.max(...e2e.sizes)} keys; ${e2e.answered.length} thumbnails answered, ${e2e.held} held in memory`)
  expect(e2e.sizes.length, 'saved thumbnails were asked for').toBeGreaterThan(0)
  expect(Math.max(...e2e.sizes), 'keys in the largest thumbs_get request').toBeLessThanOrEqual(400)
  expect(e2e.answered.length, 'more thumbnails were read than the cache holds, so evicting was exercised').toBeGreaterThan(THUMB_CACHE_CAP)
  expect(e2e.held, 'thumbnails held in memory').toBeGreaterThan(0)
  expect(e2e.held, 'thumbnails held in memory').toBeLessThanOrEqual(THUMB_CACHE_CAP)

  // Cards in the viewport show the saved thumbnail the cache has for them.
  const idsInView = () =>
    page.evaluate((cardSelector) => {
      const view = document.querySelector('.content')!.getBoundingClientRect()
      return [...document.querySelectorAll<HTMLElement>(cardSelector)]
        .filter((c) => c.getBoundingClientRect().bottom > view.top && c.getBoundingClientRect().top < view.bottom)
        .map((c) => ({ id: c.dataset.assetId!, img: c.querySelector('img.asset-img')?.getAttribute('src')?.startsWith('data:') ?? false }))
    }, CARD)
  const answered = new Set(e2e.answered)
  const withoutThumb = async () => {
    const cards = await idsInView()
    return { cards: cards.length, expected: cards.filter((c) => answered.has(c.id)).length, missing: cards.filter((c) => answered.has(c.id) && !c.img).map((c) => c.id) }
  }
  await expect.poll(async () => (await withoutThumb()).missing.length, { message: 'cards in the viewport whose saved thumbnail is not shown' }).toBe(0)
  const shown = await withoutThumb()
  expect(shown.expected, `cards in the viewport with a saved thumbnail (of ${shown.cards})`).toBeGreaterThan(0)

  // A card answered from the cache is never drawn again; the ones the cache has nothing for are.
  const reads = await page.evaluate(() => window.__e2e.readPaths)
  const drawnAgain = assets.filter((a) => answered.has(a.sha) && reads.includes(a.path)).length
  expect(drawnAgain, `cards with a saved thumbnail that were read for drawing, of ${reads.length} reads`).toBe(0)
  expect(reads.length, 'cards without a saved thumbnail are drawn').toBeGreaterThan(0)
})

test('rows outside the window show placeholder cards at the real card pitch', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  const top = await measure(page)
  await scrollTo(page, Math.round(top.scrollHeight * (0.2 + Math.random() * 0.6)))
  await settle(page)
  const m = await measure(page)

  const found = await page.evaluate((cardSelector) => {
    const grid = document.querySelector<HTMLElement>('.content .grid')!
    const style = getComputedStyle(grid)
    const before = getComputedStyle(grid, '::before')
    const g = grid.getBoundingClientRect()
    const originX = g.left + grid.clientLeft + parseFloat(before.left)
    const originY = g.top + grid.clientTop + parseFloat(before.top)
    const cards = [...grid.querySelectorAll<HTMLElement>(cardSelector.replace('.grid ', ''))].map((c) => {
      const r = c.getBoundingClientRect()
      return { left: r.left, top: r.top, opaque: getComputedStyle(c).backgroundColor !== 'rgba(0, 0, 0, 0)' }
    })
    return {
      marked: grid.hasAttribute('data-placeholders'),
      maskSize: before.getPropertyValue('mask-size') || before.getPropertyValue('-webkit-mask-size'),
      mask: before.getPropertyValue('mask-image') || before.getPropertyValue('-webkit-mask-image'),
      background: before.backgroundColor,
      colWidth: parseFloat(style.gridTemplateColumns.split(' ')[0]),
      colGap: parseFloat(style.columnGap),
      originX,
      originY,
      beforeHeight: parseFloat(before.height),
      cards,
    }
  }, CARD)

  expect(found.marked, 'grid carries the placeholder background').toBe(true)
  expect(found.mask, 'placeholder mask').toContain('svg')
  const pitchX = found.colWidth + found.colGap
  const pitchY = m.rowHeight + m.rowGap
  expect(found.maskSize.replace(/px/g, '').split(' ').map(Number), 'placeholder tile equals the card pitch').toEqual([pitchX, pitchY])
  expect(found.beforeHeight, 'placeholder layer is bounded').toBeLessThanOrEqual(m.viewHeight * 4 + Math.ceil(found.cards.length / m.columns) * pitchY)
  expect(found.background, 'placeholder colour').not.toBe('rgba(0, 0, 0, 0)')

  // Every rendered card sits on a placeholder cell, within a pixel, and covers it.
  expect(found.cards.length).toBeGreaterThan(0)
  for (const c of found.cards) {
    const col = Math.round((c.left - found.originX) / pitchX)
    const row = Math.round((c.top - found.originY) / pitchY)
    expect(Math.abs(c.left - (found.originX + col * pitchX)), `card column ${col} off its placeholder`).toBeLessThanOrEqual(1)
    expect(Math.abs(c.top - (found.originY + row * pitchY)), `card row ${row} off its placeholder`).toBeLessThanOrEqual(1)
    expect(c.opaque, 'rendered card is opaque').toBe(true)
  }

  // With the rendered cards hidden, the placeholders are what the eye sees.
  await page.addStyleTag({ content: `${CARD} { visibility: hidden !important }` })
  await settle(page)
  await page.screenshot({ path: 'e2e/screenshots/placeholder-cards-behind-the-grid.png' })
})

test('row height follows the natural card height across cell size toggles', async ({ page }) => {
  await openApp(page, 'catalog')
  await waitForCards(page)
  const naturalHeight = () =>
    page.evaluate((cardSelector) => {
      const card = document.querySelector<HTMLElement>(cardSelector)!
      const saved = card.style.minHeight
      card.style.minHeight = '0'
      const height = card.offsetHeight
      card.style.minHeight = saved
      return height
    }, CARD)
  const sizes = page.locator('.size-toggle button')
  const first = (await measure(page)).rowHeight
  expect(first, 'start: row height is the natural card height').toBe(await naturalHeight())
  for (const [name, button] of [['large', 1], ['small', 0], ['large', 1], ['small', 0]] as const) {
    await sizes.nth(button).click()
    await settle(page, 4)
    const m = await measure(page)
    expect(m.rowHeight, `${name} cells: row height equals the natural card height`).toBe(await naturalHeight())
    if (name === 'small') expect(m.rowHeight, 'small cells return to the starting row height').toBe(first)
  }
})

test('10f. importing many picked assets into Unity sends the previews the disk cache has, also those not held in memory', async ({ page }) => {
  test.setTimeout(120_000)
  await openApp(page, 'branch', { cachedThumbs: 1, unityEditor: true })
  await waitForCards(page)
  await page.keyboard.press('Control+a')
  const held = await page.evaluate(() => window.__e2eThumbCount?.() ?? -1)
  expect(held, 'thumbnails in memory are only those of the cards seen').toBeLessThan(BRANCH_COUNT / 2)
  await page.keyboard.press('Control+i')
  await expect.poll(() => page.evaluate(() => window.__e2e.exportPreviews.length), { message: 'an export was sent', timeout: 60_000 }).toBe(1)
  const previews = await page.evaluate(() => window.__e2e.exportPreviews[0])
  expect(previews.length, 'previews sent with the export').toBe(BRANCH_COUNT)
  expect(await page.evaluate(() => window.__e2eThumbCount?.() ?? -1), 'the export keeps nothing in memory').toBeLessThan(BRANCH_COUNT / 2)
})

test('10h. a scan that ends while the catalog is open still shows in the branch tab when coming back', async ({ page }) => {
  test.setTimeout(120_000)
  const scan = { perFlush: 25, intervalMs: 400, flushes: 12, of: 'branch' as const }
  const prefabs = branchAssets().filter((a) => a.type === 'prefab').length
  await openApp(page, 'branch', { effectsTab: true, factScan: scan })
  await waitForCards(page)
  const tab = page.locator('.tab', { hasText: 'VFX' })
  // The VFX tab appears when the first findings land; the scan goes on after the tab is opened and the scope is left.
  await tab.click({ timeout: 15_000 })
  const scans = () => page.evaluate(() => window.__e2e.calls.filter((c) => c === 'assets_known_assets').length)
  await page.locator('.crumb-seg .picker-trigger').click()
  await page.locator('.scope-all label').first().click()
  await page.keyboard.press('Escape')
  await expect(page.locator('.crumb-seg .picker-value')).toContainText('All repos')
  // Both scans end while the catalog is open (it holds none of the branch's prefabs).
  await expect.poll(scans, { timeout: 30_000 }).toBe(2)
  await page.waitForTimeout((scan.flushes * scan.intervalMs) + 2500)
  await page.locator('.crumb-seg .picker-trigger').click()
  await page.getByRole('button', { name: 'Only acme/alpha' }).click()
  await page.keyboard.press('Escape')
  await expect(page.locator('.crumb-seg .picker-value')).not.toContainText('All repos')
  const tabCount = tab.locator('.num')
  await expect(tabCount).toHaveText(String(prefabs))
  await expect(page.locator('.toolbar-count')).toHaveText(String(prefabs))
})

test('the placeholder layer stays small on a 375,000 asset catalog and the visible grid is always covered', async ({ page }) => {
  await openApp(page, 'catalog', { catalogCount: 375_000 })
  await waitForCards(page)
  const layer = () =>
    page.evaluate((cardSelector) => {
      const grid = document.querySelector<HTMLElement>('.content .grid')!
      const before = getComputedStyle(grid, '::before')
      const g = grid.getBoundingClientRect()
      const top = g.top + grid.clientTop + parseFloat(before.top)
      return { top, bottom: top + parseFloat(before.height), cards: grid.querySelectorAll(cardSelector.replace('.grid ', '')).length }
    }, CARD)
  const positions = [0.5, 0.9, 0.12, 0.999, 0.37, Math.random(), Math.random()]
  for (const at of positions) {
    const before = await measure(page)
    await scrollTo(page, Math.round((before.scrollHeight - before.viewHeight) * at))
    await settle(page, 4)
    const m = await measure(page)
    const l = await layer()
    const pitch = m.rowHeight + m.rowGap
    expect(l.bottom - l.top, `layer height at ${at} (${JSON.stringify(m)})`).toBeLessThanOrEqual(m.viewHeight * 4 + Math.ceil(l.cards / m.columns) * pitch)
    expectCovered(m, `position ${at}`)
    // The visible grid area lies inside the placeholder layer, so what no card covers shows a placeholder.
    expect(l.top, `layer starts above the visible area at ${at}`).toBeLessThanOrEqual(m.areaTop + 1)
    expect(l.bottom, `layer ends below the visible area at ${at}`).toBeGreaterThanOrEqual(m.areaBottom - 1)
  }
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
