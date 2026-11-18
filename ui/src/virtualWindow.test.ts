import { describe, expect, it } from 'vitest'
import { anchorItem, placeholderMask, windowRange } from './virtualWindow'

const base = { count: 1000, columns: 5, rowHeight: 100, rowGap: 10, viewTop: 0, viewHeight: 500, overscanRows: 2 }
// 200 rows, pitch 110.

describe('windowRange', () => {
  it('starts at the top with no top padding', () => {
    const r = windowRange(base)
    expect(r.start).toBe(0)
    expect(r.padTop).toBe(0)
    // ceil(500 / 110) = 5, plus 2 overscan rows = 7 rows.
    expect(r.end).toBe(35)
    expect(r.padBottom).toBe((200 - 7) * 110)
  })

  it('windows the middle with padding on both sides', () => {
    const r = windowRange({ ...base, viewTop: 11000 })
    // floor(11000 / 110) = 100, minus 2 = row 98.
    expect(r.start).toBe(98 * 5)
    expect(r.padTop).toBe(98 * 110)
    expect(r.end).toBe(Math.ceil(11500 / 110) * 5 + 10)
    expect(r.padBottom).toBeGreaterThan(0)
  })

  it('has no bottom padding at the end', () => {
    const r = windowRange({ ...base, viewTop: 200 * 110 - 500 })
    expect(r.end).toBe(1000)
    expect(r.padBottom).toBe(0)
  })

  it('renders nothing when the view is far above the grid', () => {
    const r = windowRange({ ...base, viewTop: -5000 })
    expect(r).toEqual({ start: 0, end: 0, padTop: 0, padBottom: 200 * 110 })
  })

  it('renders the head when the view just overlaps the grid from above', () => {
    const r = windowRange({ ...base, viewTop: -300 })
    expect(r.start).toBe(0)
    expect(r.end).toBeGreaterThan(0)
  })

  it('renders only the tail when the view is below the grid', () => {
    const r = windowRange({ ...base, viewTop: 1_000_000 })
    expect(r.start).toBe(1000)
    expect(r.end).toBe(1000)
    expect(r.padBottom).toBe(0)
    expect(r.padTop).toBe(200 * 110)
  })

  it('keeps start <= end <= count when the list shrinks under an old scroll position', () => {
    const r = windowRange({ ...base, count: 20, viewTop: 11000 })
    expect(r.start).toBeLessThanOrEqual(r.end)
    expect(r.end).toBeLessThanOrEqual(20)
  })

  it('never starts past the end when the last row is partial', () => {
    const r = windowRange({ ...base, count: 21, viewTop: 1_000_000 })
    expect(r.start).toBeLessThanOrEqual(r.end)
    expect(r.end).toBeLessThanOrEqual(21)
    expect(r.padBottom).toBe(0)
  })

  it('renders a fixed head before the grid is measured', () => {
    expect(windowRange({ ...base, columns: 0 })).toEqual({ start: 0, end: 60, padTop: 0, padBottom: 0 })
    expect(windowRange({ ...base, rowHeight: 0 })).toEqual({ start: 0, end: 60, padTop: 0, padBottom: 0 })
    expect(windowRange({ ...base, columns: 0, count: 10 }).end).toBe(10)
  })

  it('handles an empty list', () => {
    expect(windowRange({ ...base, count: 0 })).toEqual({ start: 0, end: 0, padTop: 0, padBottom: 0 })
  })
})

describe('anchorItem', () => {
  const view = { count: 1000, columns: 5, rowHeight: 100, rowGap: 10, viewTop: 11000, viewHeight: 500 }

  it('keeps the preferred item when its row is in view', () => {
    expect(anchorItem({ ...view, preferred: 500 + 2 })).toBe(502)
  })

  it('falls back to the first item of the first visible row', () => {
    expect(anchorItem({ ...view, preferred: 3 })).toBe(100 * 5)
    expect(anchorItem({ ...view, preferred: -1 })).toBe(100 * 5)
  })

  it('stays inside the list and returns -1 when unmeasured', () => {
    expect(anchorItem({ ...view, count: 20, preferred: -1 })).toBe(19)
    expect(anchorItem({ ...view, columns: 0, preferred: 0 })).toBe(-1)
    expect(anchorItem({ ...view, count: 0, preferred: 0 })).toBe(-1)
  })
})

describe('placeholderMask', () => {
  it('draws one tile of a card plus its gaps', () => {
    const url = decodeURIComponent(placeholderMask({ colWidth: 120, colGap: 8, rowHeight: 170, rowGap: 12 }))
    expect(url).toContain("width='128' height='182'")
    expect(url).toContain("<rect width='120' height='120'")
    expect(url.startsWith('url("data:image/svg+xml,')).toBe(true)
  })
})
