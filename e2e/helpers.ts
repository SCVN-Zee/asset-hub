import type { Page } from '@playwright/test'
import type { E2EConfig } from './harness'

export type Scope = 'catalog' | 'branch'

const CARD = '.grid .asset-card:not(.skeleton)'
export const SELECTED = `${CARD}[aria-pressed="true"]`
export { CARD }

/** Opens the app on the fake backend: the catalog scope ("All repos") or the open branch. */
export async function openApp(page: Page, scope: Scope, config: E2EConfig = {}): Promise<void> {
  page.on('console', (msg) => msg.text().startsWith('E2E-UNKNOWN') && console.log(msg.text()))
  page.on('pageerror', (err) => console.log(`PAGEERROR ${err.message}`))
  await page.addInitScript(
    ({ config, scope }) => {
      window.__e2eConfig = config
      if (scope === 'catalog') localStorage.setItem('assethub.scope', JSON.stringify({ scope: 'all', sel: { repos: 'all', branches: 'all' } }))
    },
    { config, scope }
  )
  await page.goto('/e2e/harness.html')
  await page.locator('.hub').waitFor()
}

export async function waitForCards(page: Page): Promise<void> {
  await page.locator(CARD).first().waitFor({ timeout: 20_000 })
  await settle(page)
}

/** Two animation frames, then one task, so layout and React have caught up with the last scroll or resize. */
export async function settle(page: Page, frames = 2): Promise<void> {
  await page.evaluate(
    (n) =>
      new Promise<void>((resolve) => {
        const step = (left: number) => (left === 0 ? resolve() : requestAnimationFrame(() => step(left - 1)))
        step(n)
      }),
    frames
  )
}

export interface Measure {
  cards: number
  columns: number
  rowHeight: number
  rowGap: number
  viewHeight: number
  scrollTop: number
  /** Scroll offset relative to the grid's first row (as the grid itself counts it). */
  viewTop: number
  scrollHeight: number
  /** Visible part of the grid area: below the sticky toolbar, inside the scroller. */
  areaTop: number
  areaBottom: number
  gridTop: number
  gridBottom: number
  padTop: number
  padBottom: number
  /** Largest vertical gap inside the visible area that no card covers (px). */
  gap: number
  /** Cards in the last row of the rendered cards, and the bottom of the last card. */
  lastRowCards: number
  lastCardBottom: number
  selected?: { top: number; bottom: number; underToolbar: boolean }
}

/** DOM measurements of the grid, in the page itself so they cost one round trip. */
export function measure(page: Page): Promise<Measure> {
  return page.evaluate((cardSelector) => {
    const scroller = document.querySelector<HTMLElement>('.content')!
    const grid = document.querySelector<HTMLElement>('.content .grid')!
    const toolbar = document.querySelector<HTMLElement>('.content .toolbar, .content .content-head')
    const style = getComputedStyle(grid)
    const cards = [...grid.querySelectorAll<HTMLElement>(cardSelector.replace('.grid ', ''))]
    const rects = cards.map((c) => c.getBoundingClientRect())
    const view = scroller.getBoundingClientRect()
    const g = grid.getBoundingClientRect()
    const padTop = parseFloat(style.paddingTop)
    const padBottom = parseFloat(style.paddingBottom)
    const toolbarBottom = toolbar ? toolbar.getBoundingClientRect().bottom : view.top
    const areaTop = Math.max(view.top + scroller.clientTop, toolbarBottom, g.top + padTop)
    const areaBottom = Math.min(view.top + scroller.clientTop + scroller.clientHeight, g.bottom - padBottom)
    const rowGap = parseFloat(style.rowGap) || 0
    // Vertical union of the card boxes inside the area; the largest uncovered stretch is the gap.
    const spans = rects
      .map((r) => [Math.max(r.top, areaTop), Math.min(r.bottom, areaBottom)] as const)
      .filter(([a, b]) => b > a)
      .sort((x, y) => x[0] - y[0])
    let gap = 0
    let covered = areaTop
    if (areaBottom > areaTop) {
      for (const [a, b] of spans) {
        // The space between two rows is the row gap, not a hole.
        gap = Math.max(gap, a - covered - rowGap)
        covered = Math.max(covered, b)
      }
      gap = Math.max(gap, areaBottom - covered - rowGap)
    }
    const lastBottom = Math.max(...rects.map((r) => r.bottom), 0)
    const lastRow = rects.filter((r) => Math.abs(r.bottom - lastBottom) < 2).length
    const selectedCard = grid.querySelector<HTMLElement>('.asset-card[aria-pressed="true"]')
    const sel = selectedCard?.getBoundingClientRect()
    return {
      cards: cards.length,
      columns: style.gridTemplateColumns.split(' ').filter(Boolean).length,
      rowHeight: parseFloat(style.gridAutoRows) || rects[0]?.height || 0,
      rowGap,
      viewHeight: scroller.clientHeight,
      scrollTop: scroller.scrollTop,
      viewTop: view.top + scroller.clientTop - (g.top + padTop),
      scrollHeight: scroller.scrollHeight,
      areaTop,
      areaBottom,
      gridTop: g.top,
      gridBottom: g.bottom,
      padTop,
      padBottom,
      gap,
      lastRowCards: lastRow,
      lastCardBottom: lastBottom,
      selected: sel ? { top: sel.top, bottom: sel.bottom, underToolbar: sel.top < toolbarBottom - 1 } : undefined
    }
  }, CARD)
}

export function scrollTo(page: Page, top: number | 'end'): Promise<void> {
  return page.evaluate((to) => {
    const scroller = document.querySelector<HTMLElement>('.content')!
    scroller.scrollTop = to === 'end' ? scroller.scrollHeight : to
  }, top)
}

/** Selected card inside the scroller's visible box (the sticky toolbar is reported apart). */
export function selectedInView(m: Measure, viewTop: number): boolean {
  return !!m.selected && m.selected.top >= viewTop - 1 && m.selected.bottom <= viewTop + m.viewHeight + 1
}

/** The scroller's top edge in viewport coordinates. */
export function scrollerTop(page: Page): Promise<number> {
  return page.evaluate(() => {
    const s = document.querySelector<HTMLElement>('.content')!
    return s.getBoundingClientRect().top + s.clientTop
  })
}
