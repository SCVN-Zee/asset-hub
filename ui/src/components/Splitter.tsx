import { useState, type KeyboardEvent, type PointerEvent } from 'react'

/** A size someone dragged to, kept on this machine; undefined until then (the layout's default applies). */
export function useStoredSize(key: string): [number | undefined, (px: number | undefined) => void] {
  const [size, setSize] = useState<number | undefined>(() => {
    try {
      const v = Number(localStorage.getItem(key))
      return v > 0 ? v : undefined
    } catch {
      return undefined
    }
  })
  const store = (px: number | undefined) => {
    setSize(px)
    try {
      if (px === undefined) localStorage.removeItem(key)
      else localStorage.setItem(key, String(Math.round(px)))
    } catch {
      // Not kept for next time; the size still applies now.
    }
  }
  return [size, store]
}

interface Props {
  /** `x` resizes a width (dragged sideways), `y` a height. */
  axis: 'x' | 'y'
  /** The element it resizes, beside it: its size now is where a drag starts. */
  target: 'prev' | 'next'
  min: number
  max: () => number
  onResize(px: number | undefined): void
  label: string
  className?: string
}

/**
 * A bar dragged to resize the panel beside it; arrow keys nudge it, a
 * double-click puts the default back.
 */
export function Splitter({ axis, target, min, max, onResize, label, className }: Props) {
  // Dragging a panel on the right wider means moving left.
  const sign = axis === 'x' && target === 'next' ? -1 : 1
  const sizeOf = (el: Element) => {
    const sibling = target === 'prev' ? el.previousElementSibling : el.nextElementSibling
    const box = sibling?.getBoundingClientRect()
    return box ? (axis === 'x' ? box.width : box.height) : min
  }
  const clamp = (px: number) => Math.round(Math.min(Math.max(px, min), Math.max(min, max())))

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    e.preventDefault()
    const bar = e.currentTarget
    bar.setPointerCapture(e.pointerId)
    const from = axis === 'x' ? e.clientX : e.clientY
    const start = sizeOf(bar)
    const move = (ev: globalThis.PointerEvent) => onResize(clamp(start + sign * ((axis === 'x' ? ev.clientX : ev.clientY) - from)))
    const end = () => {
      bar.removeEventListener('pointermove', move)
      bar.removeEventListener('pointerup', end)
      bar.removeEventListener('pointercancel', end)
      document.body.classList.remove('resizing')
    }
    bar.addEventListener('pointermove', move)
    bar.addEventListener('pointerup', end)
    bar.addEventListener('pointercancel', end)
    document.body.classList.add('resizing')
  }

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const keys = axis === 'x' ? { ArrowLeft: -sign, ArrowRight: sign } : { ArrowUp: -1, ArrowDown: 1 }
    const dir = keys[e.key as keyof typeof keys]
    if (!dir) return
    e.preventDefault()
    onResize(clamp(sizeOf(e.currentTarget) + dir * (e.shiftKey ? 64 : 16)))
  }

  return (
    <div
      role="separator"
      aria-orientation={axis === 'x' ? 'vertical' : 'horizontal'}
      aria-label={label}
      title={label}
      tabIndex={0}
      className={`splitter splitter-${axis}${className ? ` ${className}` : ''}`}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      onDoubleClick={() => onResize(undefined)}
    />
  )
}
