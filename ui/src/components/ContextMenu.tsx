import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { ChevronRightIcon } from './Icons'

/** One line of a right-click menu: an action, a submenu, or a separator ('-'). */
export type MenuItem =
  | '-'
  | {
      label: string
      icon?: ReactNode
      /** Shown on the right, e.g. a shortcut or a count. */
      hint?: string
      disabled?: boolean
      /** Drawn in red: removes or stops something. */
      danger?: boolean
      /** Ticked (a collection it is in, a mode that is on). */
      checked?: boolean
      onSelect?(): void
      submenu?: MenuItem[]
    }

export interface MenuState {
  x: number
  y: number
  items: MenuItem[]
}

/** What a component holding a menu keeps: open one at the pointer of a right-click. */
export function useContextMenu() {
  const [menu, setMenu] = useState<MenuState>()
  const open = (e: { clientX: number; clientY: number; preventDefault(): void; stopPropagation(): void }, items: MenuItem[]) => {
    e.preventDefault()
    e.stopPropagation()
    if (items.length) setMenu({ x: e.clientX, y: e.clientY, items })
  }
  return { menu, open, close: () => setMenu(undefined), element: menu ? <ContextMenu {...menu} onClose={() => setMenu(undefined)} /> : null }
}

const EDGE = 8

/** A list of actions at a point, kept inside the window; closes on a pick, a click outside, Escape, scrolling or leaving the window. */
export function ContextMenu({ x, y, items, onClose, sub, backX, onBack }: MenuState & { onClose(): void; sub?: boolean; backX?: number; onBack?(): void }) {
  const box = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number }>()
  const [openSub, setOpenSub] = useState<number>()
  const [subAt, setSubAt] = useState<{ x: number; y: number; back: number }>()

  useLayoutEffect(() => {
    const el = box.current
    if (!el) return
    const w = el.offsetWidth
    const h = el.offsetHeight
    // No room on the right: a menu moves left; a submenu opens on the left of its line.
    const left = x + w + EDGE <= window.innerWidth ? x : sub && backX !== undefined ? backX - w : window.innerWidth - w - EDGE
    setPos({ left: Math.max(EDGE, left), top: Math.max(EDGE, Math.min(y, window.innerHeight - h - EDGE)) })
  }, [x, y, sub, backX])

  const showSub = (i: number, el: HTMLElement) => {
    const r = el.getBoundingClientRect()
    setSubAt({ x: r.right + 2, y: r.top - 5, back: r.left - 2 })
    setOpenSub(i)
  }

  useEffect(() => {
    if (sub) return
    const outside = (e: PointerEvent) => !(e.target as HTMLElement).closest?.('.context-menu') && onClose()
    const key = (e: KeyboardEvent) => e.key === 'Escape' && onClose()
    window.addEventListener('pointerdown', outside, true)
    window.addEventListener('keydown', key)
    window.addEventListener('blur', onClose)
    window.addEventListener('resize', onClose)
    window.addEventListener('wheel', onClose, { passive: true })
    return () => {
      window.removeEventListener('pointerdown', outside, true)
      window.removeEventListener('keydown', key)
      window.removeEventListener('blur', onClose)
      window.removeEventListener('resize', onClose)
      window.removeEventListener('wheel', onClose)
    }
  }, [sub, onClose])

  // Focus the first action, so arrows and Enter work at once.
  useEffect(() => {
    if (!sub) box.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
  }, [sub])

  const move = (e: React.KeyboardEvent) => {
    const buttons = [...(box.current?.querySelectorAll<HTMLButtonElement>(':scope > button:not(:disabled)') ?? [])]
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement)
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      e.stopPropagation()
      buttons[(i + (e.key === 'ArrowDown' ? 1 : buttons.length - 1)) % buttons.length]?.focus()
    } else if (e.key === 'ArrowRight') {
      e.stopPropagation()
      const el = document.activeElement as HTMLButtonElement | null
      const i = Number(el?.dataset.index)
      if (el && typeof items[i] === 'object' && (items[i] as { submenu?: MenuItem[] }).submenu) {
        showSub(i, el)
        // Into the submenu once it is drawn.
        requestAnimationFrame(() => box.current?.querySelector<HTMLButtonElement>('.context-menu button:not(:disabled)')?.focus())
      }
    } else if (e.key === 'ArrowLeft' && sub) {
      e.stopPropagation()
      onBack?.()
    }
  }

  const menu = (
    <div className="context-menu" role="menu" ref={box} style={pos ?? { left: x, top: y, visibility: 'hidden' }} onKeyDown={move} onContextMenu={(e) => e.preventDefault()}>
      {items.map((item, i) =>
        item === '-' ? (
          <div key={i} className="context-sep" role="separator" />
        ) : (
          <button
            key={i}
            type="button"
            data-index={i}
            role={item.checked !== undefined ? 'menuitemcheckbox' : 'menuitem'}
            aria-checked={item.checked}
            aria-haspopup={item.submenu ? 'menu' : undefined}
            aria-expanded={item.submenu ? openSub === i : undefined}
            className={`context-item${item.danger ? ' danger' : ''}`}
            disabled={item.disabled || (item.submenu && !item.submenu.length)}
            onMouseEnter={(e) => (item.submenu ? showSub(i, e.currentTarget) : setOpenSub(undefined))}
            onClick={(e) => {
              if (item.submenu) return showSub(i, e.currentTarget)
              onClose()
              item.onSelect?.()
            }}
          >
            <span className="context-check" aria-hidden="true">
              {item.checked ? '✓' : item.icon}
            </span>
            <span className="context-label">{item.label}</span>
            {item.hint && <span className="context-hint">{item.hint}</span>}
            {item.submenu && <ChevronRightIcon size={12} />}
          </button>
        )
      )}
      {openSub !== undefined && subAt && typeof items[openSub] === 'object' && (items[openSub] as { submenu?: MenuItem[] }).submenu && (
        <ContextMenu
          x={subAt.x}
          y={subAt.y}
          backX={subAt.back}
          items={(items[openSub] as { submenu: MenuItem[] }).submenu}
          onClose={onClose}
          onBack={() => {
            const at = openSub
            setOpenSub(undefined)
            box.current?.querySelector<HTMLButtonElement>(`:scope > button[data-index="${at}"]`)?.focus()
          }}
          sub
        />
      )}
    </div>
  )
  return sub ? menu : createPortal(menu, document.body)
}
