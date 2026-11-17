export interface WindowInput {
  count: number
  columns: number
  /** Height of one row, without the gap below it. */
  rowHeight: number
  rowGap: number
  /** Scroll offset relative to the grid's first row. May be negative or past the end. */
  viewTop: number
  viewHeight: number
  overscanRows: number
}

export interface WindowRange {
  start: number
  end: number
  /** Extra padding that stands in for the rows left out above the window. */
  padTop: number
  /** Extra padding that stands in for the rows left out below the window. */
  padBottom: number
}

/** Items drawn on the first paint, before the grid has been measured. */
const UNMEASURED_COUNT = 60

const clamp = (n: number, lo: number, hi: number) => Math.min(Math.max(n, lo), hi)

/** Which items of a fixed-row grid to render for the current scroll position. */
export function windowRange(input: WindowInput): WindowRange {
  const { count, columns, rowHeight, rowGap, viewTop, viewHeight, overscanRows } = input
  if (columns < 1 || rowHeight <= 0) return { start: 0, end: Math.min(count, UNMEASURED_COUNT), padTop: 0, padBottom: 0 }
  const pitch = rowHeight + rowGap
  const rows = Math.ceil(count / columns)
  const firstRow = clamp(Math.floor(viewTop / pitch) - overscanRows, 0, rows)
  const lastRow = clamp(Math.ceil((viewTop + viewHeight) / pitch) + overscanRows, firstRow, rows)
  return {
    start: Math.min(count, firstRow * columns),
    end: Math.min(count, lastRow * columns),
    padTop: firstRow * pitch,
    padBottom: (rows - lastRow) * pitch,
  }
}

/**
 * The item to keep in place when the column count changes: `preferred` when its row is in view,
 * otherwise the first item of the first row in view. `-1` when the grid cannot be measured.
 */
export function anchorItem(input: Pick<WindowInput, 'count' | 'columns' | 'rowHeight' | 'rowGap' | 'viewTop' | 'viewHeight'> & { preferred: number }): number {
  const { count, columns, rowHeight, rowGap, viewTop, viewHeight, preferred } = input
  if (count < 1 || columns < 1 || rowHeight <= 0) return -1
  const pitch = rowHeight + rowGap
  if (preferred >= 0 && preferred < count) {
    const top = Math.floor(preferred / columns) * pitch
    if (top + rowHeight > viewTop && top < viewTop + viewHeight) return preferred
  }
  const row = Math.max(0, Math.floor(viewTop / pitch))
  return Math.min(count - 1, row * columns)
}
