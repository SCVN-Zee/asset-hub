import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react'
import { ChevronRightIcon } from './Icons'

export interface DropdownOption<T extends string> {
  value: T
  label: string
  /** Muted text after the label, e.g. a count. */
  hint?: string
}

interface Props<T extends string> {
  value: T
  options: readonly DropdownOption<T>[]
  onChange(value: T): void
  /** Accessible name; also the tooltip unless `title` is given. */
  label: string
  title?: string
  id?: string
  size?: 'md' | 'sm' | 'xs'
  /** Extra classes for the trigger button (width, overlay styling…). */
  className?: string
  disabled?: boolean
}

/** Styled replacement for `<select>`, matching `Picker` without its search box. */
export function Dropdown<T extends string>({ value, options, onChange, label, title, id, size = 'md', className = '', disabled }: Props<T>) {
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  const [pos, setPos] = useState<CSSProperties>()
  const root = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const pop = useRef<HTMLUListElement>(null)
  const current = options.find((o) => o.value === value)

  // Fixed, so viewers with overflow: hidden do not clip it; opens upwards near the bottom.
  useLayoutEffect(() => {
    if (!open || !trigger.current) return
    const r = trigger.current.getBoundingClientRect()
    const below = window.innerHeight - r.bottom
    const up = below < 220 && r.top > below
    setPos({
      minWidth: r.width,
      ...(r.left > window.innerWidth / 2 ? { right: window.innerWidth - r.right } : { left: r.left }),
      ...(up ? { bottom: window.innerHeight - r.top + 6, maxHeight: r.top - 16 } : { top: r.bottom + 6, maxHeight: below - 16 })
    })
    setActive(Math.max(0, options.findIndex((o) => o.value === value)))
  }, [open])

  useEffect(() => {
    if (!open) return
    const close = () => setOpen(false)
    const outside = (e: Event) => {
      if (!root.current?.contains(e.target as Node)) close()
    }
    document.addEventListener('mousedown', outside)
    document.addEventListener('scroll', outside, true)
    window.addEventListener('resize', close)
    return () => {
      document.removeEventListener('mousedown', outside)
      document.removeEventListener('scroll', outside, true)
      window.removeEventListener('resize', close)
    }
  }, [open])

  useEffect(() => {
    pop.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [active, pos])

  const choose = (option: T) => {
    setOpen(false)
    if (option !== value) onChange(option)
  }

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!open) {
      if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
      setOpen(true)
    } else if (e.key === 'ArrowDown') setActive((i) => Math.min(options.length - 1, i + 1))
    else if (e.key === 'ArrowUp') setActive((i) => Math.max(0, i - 1))
    else if (e.key === 'Enter' && options[active]) choose(options[active].value)
    else if (e.key === 'Escape' || e.key === 'Tab') setOpen(false)
    else return
    if (e.key !== 'Tab') e.preventDefault()
  }

  return (
    <div className="dropdown" ref={root} onKeyDown={onKeyDown}>
      <button
        ref={trigger}
        id={id}
        type="button"
        className={`select picker-trigger ${size === 'md' ? '' : `select-${size}`} ${className}`}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={label}
        title={title ?? current?.label ?? label}
        disabled={disabled || !options.length}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="picker-value">{current?.label ?? value}</span>
        <span className="picker-caret" data-open={open}>
          <ChevronRightIcon />
        </span>
      </button>
      {open && pos && (
        <ul className="dropdown-pop" role="listbox" aria-label={label} ref={pop} style={pos}>
          {options.map((o, index) => (
            <li
              key={o.value}
              role="option"
              aria-selected={o.value === value}
              data-active={index === active}
              className="picker-option"
              onMouseEnter={() => setActive(index)}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => choose(o.value)}
            >
              <span className="picker-option-text">{o.label}</span>
              {o.hint && <span className="muted small">{o.hint}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
