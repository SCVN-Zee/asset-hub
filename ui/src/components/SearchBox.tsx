import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Suggestion } from '@shared/types'
import { parseAssetLink } from '@shared/deeplink'
import { SearchIcon } from './Icons'

interface Props {
  value: string
  /** Keyword suggestions for what is typed; answered after a round trip. */
  suggest(text: string): Promise<Suggestion[]>
  onChange(value: string): void
}

/** Search field with keyword suggestions (groups like "fish", and words used in file names). */
export function SearchBox({ value, suggest, onChange }: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  /** Highlighted suggestion; -1 while none is (typing, or just focused): Enter then keeps what was typed. */
  const [active, setActive] = useState(-1)
  const [suggestions, setSuggestions] = useState<Suggestion[]>([])
  // One question at a time: the last answer stays while the newest is asked, so typing never waits.
  const asking = useRef({ busy: false, next: undefined as string | undefined, open: false, mounted: true })
  useEffect(() => {
    asking.current.mounted = true
    return () => {
      asking.current.mounted = false
    }
  }, [])
  useEffect(() => {
    const state = asking.current
    state.open = open
    if (!open) {
      state.next = undefined
      setSuggestions([])
      return
    }
    state.next = value
    if (state.busy) return
    state.busy = true
    void (async () => {
      try {
        for (let text = state.next; text !== undefined; text = state.next) {
          state.next = undefined
          const answer = await suggest(text).catch(() => [])
          if (state.mounted && state.open) setSuggestions(answer)
        }
      } finally {
        state.busy = false
      }
    })()
  }, [open, suggest, value])
  const choose = (text: string) => {
    onChange(text)
    setOpen(false)
  }

  return (
    <div className="search search-box">
      <SearchIcon />
      <label htmlFor="q" className="sr-only">
        {t('browse.search')}
      </label>
      <input
        id="q"
        type="search"
        autoComplete="off"
        role="combobox"
        aria-expanded={suggestions.length > 0}
        aria-controls="q-suggestions"
        placeholder={t('browse.searchPlaceholder')}
        value={value}
        onChange={(e) => {
          onChange(e.target.value)
          setActive(-1)
          // A pasted assethub:// link opens its asset and empties the box: no suggestions for Enter to pick.
          setOpen(!parseAssetLink(e.target.value))
        }}
        onFocus={() => {
          setOpen(true)
          setActive(-1)
        }}
        onBlur={() => setOpen(false)}
        onKeyDown={(e) => {
          // Enter searches what was typed and closes the list, even before its suggestions have come in.
          if (e.key === 'Enter' && (active < 0 || !suggestions[active])) {
            setOpen(false)
            return
          }
          if (!suggestions.length) return
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault()
            // From none highlighted, Down goes to the first and Up to the last.
            setActive((i) => (e.key === 'ArrowDown' ? (i + 1) % suggestions.length : i <= 0 ? suggestions.length - 1 : i - 1))
          } else if (e.key === 'Enter') {
            // Enter searches what was typed; a suggestion only when one was picked with the arrows (or the mouse).
            e.preventDefault()
            if (active >= 0 && suggestions[active]) choose(suggestions[active].text)
            else setOpen(false)
          } else if (e.key === 'Tab' && !e.shiftKey) {
            // Tab completes: the highlighted suggestion, else the first.
            e.preventDefault()
            choose(suggestions[Math.max(0, active)]?.text ?? value)
          } else if (e.key === 'Escape') setOpen(false)
        }}
      />
      {suggestions.length > 0 && (
        <ul id="q-suggestions" className="search-suggestions" role="listbox">
          {suggestions.map((s, i) => (
            <li
              key={s.text}
              role="option"
              aria-selected={i === active}
              className="picker-option"
              data-active={i === active}
              // mousedown: runs before the input loses focus and closes the list.
              onMouseDown={(e) => {
                e.preventDefault()
                choose(s.text)
              }}
              onMouseEnter={() => setActive(i)}
            >
              <span className="picker-option-text">
                {s.text}
                {s.examples?.length ? <span className="muted"> · {t('search.includes', { names: s.examples.join(', ') })}</span> : null}
              </span>
              <span className="muted small">{s.count}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
