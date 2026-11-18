import { Fragment, forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type ForwardedRef, type ReactElement, type ReactNode } from 'react'
import { anchorItem, placeholderMask, windowRange } from '../virtualWindow'
import './VirtualGrid.css'

export interface VirtualGridHandle {
  /** Scroll so the row that holds `index` is just visible. */
  scrollToIndex(index: number): void
}

interface VirtualGridProps<T> {
  items: readonly T[]
  itemKey(item: T): string
  /** `index` is the absolute position in `items`, not the position in the window. */
  renderItem(item: T, index: number): ReactNode
  overscanRows?: number
  /** Index of the item to keep in place when the columns or row height change, e.g. the selection. -1 or unset: the first item in view. */
  anchorIndex?: number
}

interface Metrics {
  columns: number
  rowGap: number
  rowHeight: number
  /** Width of one column track, the gap between columns and the grid's left padding. */
  colWidth: number
  colGap: number
  padLeft: number
  viewTop: number
  viewHeight: number
}

const EMPTY: Metrics = { columns: 0, rowGap: 0, rowHeight: 0, colWidth: 0, colGap: 0, padLeft: 0, viewTop: 0, viewHeight: 0 }

/** The nearest ancestor that scrolls vertically. */
function scrollParentOf(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    const overflow = getComputedStyle(p).overflowY
    if (overflow === 'auto' || overflow === 'scroll') return p
  }
  return null
}

/**
 * A `.grid` that keeps only the rows near the viewport in the page. The rows left out are
 * stood in for by extra padding, so the scroll height does not change. Every row gets the
 * height of the tallest card seen so far, which is what lets the window be computed.
 */
function VirtualGridInner<T>({ items, itemKey, renderItem, overscanRows = 3, anchorIndex = -1 }: VirtualGridProps<T>, ref: ForwardedRef<VirtualGridHandle>) {
  const gridRef = useRef<HTMLDivElement>(null)
  const basePad = useRef<{ top: number; bottom: number } | null>(null)
  const trackWidth = useRef('')
  const tallest = useRef(0)
  const [metrics, setMetrics] = useState<Metrics>(EMPTY)
  const latest = useRef({ count: items.length, anchorIndex })
  latest.current = { count: items.length, anchorIndex }
  const lastMetrics = useRef<Metrics>(EMPTY)
  /** The item to hold in place across a column-count change, and where its row sat in the view. */
  const pending = useRef<{ index: number; offset: number } | null>(null)

  const measure = useCallback(() => {
    const grid = gridRef.current
    if (!grid) return
    const style = getComputedStyle(grid)
    if (!basePad.current) basePad.current = { top: parseFloat(style.paddingTop) || 0, bottom: parseFloat(style.paddingBottom) || 0 }
    const tracks = style.gridTemplateColumns.split(' ').filter(Boolean)
    const columns = tracks.length
    const rowGap = parseFloat(style.rowGap) || 0
    const colWidth = parseFloat(tracks[0] ?? '') || 0
    const colGap = parseFloat(style.columnGap) || 0
    const padLeft = parseFloat(style.paddingLeft) || 0

    // A new column width means new card sizes. The cards in the page are already laid out at
    // the new width, so the row height is taken from them at once: the padding never collapses.
    if (tracks[0] !== trackWidth.current) {
      trackWidth.current = tracks[0] ?? ''
      tallest.current = 0
    }
    for (const card of grid.querySelectorAll<HTMLElement>('.asset-card')) tallest.current = Math.max(tallest.current, card.offsetHeight)

    const scroller = scrollParentOf(grid)
    let viewTop = 0
    let viewHeight = 0
    if (scroller) {
      const outer = scroller.getBoundingClientRect()
      viewTop = outer.top + scroller.clientTop - (grid.getBoundingClientRect().top + basePad.current.top)
      viewHeight = scroller.clientHeight
    }
    const prev = lastMetrics.current
    if (scroller && prev.columns > 0 && prev.rowHeight > 0 && (columns !== prev.columns || tallest.current !== prev.rowHeight) && !pending.current) {
      // The column count or the row height changed: note which item to keep in place, and where its row is in the view.
      const index = anchorItem({ count: latest.current.count, columns: prev.columns, rowHeight: prev.rowHeight, rowGap: prev.rowGap, viewTop, viewHeight, preferred: latest.current.anchorIndex })
      if (index >= 0) {
        const offset = Math.floor(index / prev.columns) * (prev.rowHeight + prev.rowGap) - viewTop
        // A selected item that was partly out of view comes back fully in view when it fits.
        const fits = index === latest.current.anchorIndex && tallest.current <= viewHeight
        pending.current = { index, offset: fits ? Math.min(Math.max(offset, 0), viewHeight - tallest.current) : offset }
      }
    } else if (scroller && pending.current && columns === prev.columns && tallest.current === prev.rowHeight) {
      // The new padding is in place: put the anchor's row back where it was.
      const { index, offset } = pending.current
      pending.current = null
      const delta = Math.floor(index / columns) * (tallest.current + rowGap) - offset - viewTop
      if (Math.abs(delta) >= 1) {
        scroller.scrollTop += delta
        viewTop += delta
      }
    }
    const next: Metrics = { columns, rowGap, rowHeight: tallest.current, colWidth, colGap, padLeft, viewTop, viewHeight }
    lastMetrics.current = next
    setMetrics((m) => (m.columns === next.columns && m.rowGap === next.rowGap && m.rowHeight === next.rowHeight && m.colWidth === next.colWidth && m.colGap === next.colGap && m.padLeft === next.padLeft && m.viewTop === next.viewTop && m.viewHeight === next.viewHeight ? m : next))
  }, [])

  // Cheap when nothing changed: setMetrics returns the old object, so this does not loop.
  useLayoutEffect(measure)

  useEffect(() => {
    const grid = gridRef.current
    if (!grid) return
    const scroller = scrollParentOf(grid)
    let frame = 0
    const schedule = () => {
      if (frame) return
      frame = requestAnimationFrame(() => {
        frame = 0
        measure()
      })
    }
    scroller?.addEventListener('scroll', schedule, { passive: true })
    const observer = new ResizeObserver(schedule)
    observer.observe(grid)
    if (scroller) observer.observe(scroller)
    return () => {
      scroller?.removeEventListener('scroll', schedule)
      observer.disconnect()
      if (frame) cancelAnimationFrame(frame)
    }
  }, [measure])

  useImperativeHandle(
    ref,
    () => ({
      scrollToIndex(index) {
        const grid = gridRef.current
        const scroller = grid && scrollParentOf(grid)
        if (!grid || !scroller || !basePad.current || metrics.columns < 1 || metrics.rowHeight <= 0) return
        const pitch = metrics.rowHeight + metrics.rowGap
        const rowTop = grid.getBoundingClientRect().top + basePad.current.top + Math.floor(index / metrics.columns) * pitch
        const rowBottom = rowTop + metrics.rowHeight
        const viewTop = scroller.getBoundingClientRect().top + scroller.clientTop
        const viewBottom = viewTop + scroller.clientHeight
        if (rowTop < viewTop) scroller.scrollTop -= viewTop - rowTop
        else if (rowBottom > viewBottom) scroller.scrollTop += rowBottom - viewBottom
      },
    }),
    [metrics],
  )

  const { start, end, padTop, padBottom } = useMemo(() => windowRange({ count: items.length, ...metrics, overscanRows }), [items.length, metrics, overscanRows])
  const base = basePad.current
  // Placeholder cards drawn by the grid itself at the measured pitch, so rows outside the window look like cards.
  // The layer covers only the rendered rows plus one viewport of rows on each side, always starting on a row
  // boundary so its tile lines up with real cards. It changes only when the window's rows change, not per scroll event,
  // and it stays far smaller than the full scroll height, which can reach the browser's layout limit.
  const { columns, rowGap, rowHeight, colWidth, colGap, padLeft, viewHeight } = metrics
  const placeholders = useMemo(() => {
    if (!base || items.length < 1 || columns < 1 || rowHeight <= 0 || colWidth <= 0) return undefined
    const pitchX = colWidth + colGap
    const pitchY = rowHeight + rowGap
    const spare = Math.ceil(viewHeight / pitchY)
    const firstRow = Math.max(0, Math.floor(start / columns) - spare)
    // A partly filled final row gets no placeholders: its empty cells would show cards that never load.
    const complete = Math.floor(items.length / columns)
    const lastRow = Math.min(complete, Math.ceil(end / columns) + spare)
    return {
      '--ph-mask': placeholderMask({ colWidth, colGap, rowHeight, rowGap }),
      '--ph-size': `${pitchX}px ${pitchY}px`,
      '--ph-left': `${padLeft}px`,
      '--ph-top': `${base.top + firstRow * pitchY}px`,
      '--ph-width': `${columns * pitchX - colGap}px`,
      '--ph-height': `${Math.max(0, (lastRow - firstRow) * pitchY - rowGap)}px`,
    }
  }, [base, items.length, start, end, viewHeight, columns, rowGap, rowHeight, colWidth, colGap, padLeft])
  const style = {
    ...placeholders,
    paddingTop: base ? base.top + padTop : undefined,
    paddingBottom: base ? base.bottom + padBottom : undefined,
    gridAutoRows: metrics.rowHeight ? `${metrics.rowHeight}px` : undefined,
    // Cards keep their own height instead of stretching to the row, so a card that grows can raise the row height.
    alignItems: 'start',
    // The grid places its own rows; the browser's scroll anchoring would shift the scroll on top of that.
    overflowAnchor: 'none' as const,
  }

  const shown: ReactNode[] = []
  for (let i = start; i < end; i++) {
    const item = items[i]
    shown.push(<Fragment key={itemKey(item)}>{renderItem(item, i)}</Fragment>)
  }
  return (
    <div className="grid" ref={gridRef} style={style} data-placeholders={placeholders ? '' : undefined}>
      {shown}
    </div>
  )
}

export const VirtualGrid = forwardRef(VirtualGridInner) as <T>(props: VirtualGridProps<T> & { ref?: ForwardedRef<VirtualGridHandle> }) => ReactElement | null
