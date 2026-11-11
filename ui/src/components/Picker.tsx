import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronRightIcon, SearchIcon } from './Icons'

interface Props {
  label: string
  value: string
  options: readonly string[]
  /** Group prefix of an option (e.g. 'vn/'), stripped from the option text; '' groups come first under `ungroupedLabel`. */
  groupOf(option: string): string
  ungroupedLabel: string
  /** Short tag shown next to an option, e.g. "default". */
  tagOf?(option: string): string | undefined
  placeholder: string
  width?: number
  onChange(value: string): void
}

interface Group {
  key: string
  items: string[]
}

/** Dropdown that always lists every option, grouped, with a search box. */
export function Picker({ label, value, options, groupOf, ungroupedLabel, tagOf, placeholder, width = 260, onChange }: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const root = useRef<HTMLDivElement>(null)
  const list = useRef<HTMLUListElement>(null)

  const groups = useMemo<Group[]>(() => {
    const q = query.trim().toLowerCase()
    const map = new Map<string, string[]>()
    for (const option of options) {
      if (q && !option.toLowerCase().includes(q)) continue
      const key = groupOf(option)
      const items = map.get(key) ?? []
      items.push(option)
      map.set(key, items)
    }
    return [...map].map(([key, items]) => ({ key, items })).sort((a, b) => (a.key === '' ? -1 : b.key === '' ? 1 : a.key.localeCompare(b.key)))
  }, [options, query, groupOf])
  const flat = useMemo(() => groups.flatMap((g) => g.items), [groups])

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', close)
    return () => document.removeEventListener('mousedown', close)
  }, [open])

  useEffect(() => {
    // Reset the highlight when opening or filtering, not on every list change.
    if (open) setActive(Math.max(0, flat.indexOf(value)))
  }, [open, query])

  useEffect(() => {
    list.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [active, open])

  const choose = (option: string) => {
    setOpen(false)
    setQuery('')
    if (option !== value) onChange(option)
  }

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') setActive((i) => Math.min(flat.length - 1, i + 1))
    else if (e.key === 'ArrowUp') setActive((i) => Math.max(0, i - 1))
    else if (e.key === 'Enter' && flat[active]) choose(flat[active])
    else if (e.key === 'Escape') setOpen(false)
    else return
    e.preventDefault()
  }

  return (
    <div className="picker" ref={root}>
      <span className="muted small">{label}</span>
      <button
        type="button"
        className="select picker-trigger"
        style={{ width }}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        disabled={!options.length}
        title={value}
      >
        <span className="picker-value">{value || placeholder}</span>
        <span className="picker-caret" data-open={open}>
          <ChevronRightIcon />
        </span>
      </button>
      {open && (
        <div className="picker-pop" onKeyDown={onKeyDown}>
          <div className="search">
            <SearchIcon />
            <input
              autoFocus
              aria-label={t('picker.search', { label })}
              placeholder={t('picker.search', { label })}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            <span className="muted small">{flat.length}</span>
          </div>
          <ul className="picker-list" role="listbox" aria-label={label} ref={list}>
            {groups.map((g) => (
              <li key={g.key || '__none'} role="presentation">
                <div className="picker-group">
                  <span>{g.key || ungroupedLabel}</span>
                  <span>{g.items.length}</span>
                </div>
                <ul role="presentation">
                  {g.items.map((option) => {
                    const index = flat.indexOf(option)
                    const tag = tagOf?.(option)
                    return (
                      <li
                        key={option}
                        role="option"
                        aria-selected={option === value}
                        data-active={index === active}
                        className="picker-option"
                        onMouseEnter={() => setActive(index)}
                        onMouseDown={(e) => e.preventDefault()}
                        onClick={() => choose(option)}
                      >
                        <span className="picker-option-text">{g.key ? option.slice(g.key.length) : option}</span>
                        {tag && <span className="badge">{tag}</span>}
                      </li>
                    )
                  })}
                </ul>
              </li>
            ))}
            {!flat.length && <li className="muted small picker-empty">{t('picker.noMatch')}</li>}
          </ul>
        </div>
      )}
    </div>
  )
}
