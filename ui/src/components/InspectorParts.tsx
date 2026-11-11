import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { useTranslation } from 'react-i18next'
import type { BudgetIssue, PlayableBudget, StatsSummary } from '@shared/types'
import { PLAYABLE_BUDGET, TIGHT_BUDGET } from '../playable'
import { formatBytes } from '../format'
import { formatDuration } from '../viewer/audio'
import { CheckIcon, MoreIcon, WarnIcon } from './Icons'

// The pieces both inspectors (a branch asset and a catalog result) are built from.

type Translate = (key: string, options?: Record<string, unknown>) => string

/** One budget issue as a sentence. */
export function issueText(t: Translate, i: BudgetIssue): string {
  const bytes = i.key.endsWith('Bytes')
  return t(`budget.${i.key}`, { value: bytes ? formatBytes(i.value) : i.value.toLocaleString(), limit: bytes ? formatBytes(i.limit) : i.limit.toLocaleString() })
}

/** Copies text and says so for a moment; a refused copy says so too, never silently. */
export function useCopy() {
  const [state, setState] = useState<'copied' | 'failed'>()
  const timer = useRef<number>()
  useEffect(() => () => window.clearTimeout(timer.current), [])
  const copy = useCallback(async (text: string) => {
    let next: 'copied' | 'failed' = 'copied'
    try {
      await navigator.clipboard.writeText(text)
    } catch {
      next = 'failed'
    }
    setState(next)
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => setState(undefined), 2200)
  }, [])
  return { state, copy }
}

/** The result of the last copy, as text. Takes no room until there is something to say. */
export function CopyStatus({ state }: { state: 'copied' | 'failed' | undefined }) {
  const { t } = useTranslation()
  return (
    <span className="copy-status" role="status" data-state={state}>
      {state === 'copied' && (
        <>
          <CheckIcon size={12} />
          {t('details.copied')}
        </>
      )}
      {state === 'failed' && (
        <>
          <WarnIcon size={12} />
          {t('inspector.copyFailed')}
        </>
      )}
    </span>
  )
}

/** The viewing condition, stated: a grey ball, a chrome ball and a colour checker under the stage. Decoration except its label. */
export function CalibrationStrip({ dark }: { dark?: boolean }) {
  const { t } = useTranslation()
  return (
    <div className="calibration">
      <i className="cal-ball" aria-hidden="true" />
      <i className="cal-ball cal-chrome" aria-hidden="true" />
      <span className="cal-patches" aria-hidden="true">
        <i style={{ background: 'var(--act)' }} />
        <i style={{ background: 'var(--type-fbx)' }} />
        <i style={{ background: 'var(--type-audio)' }} />
        <i style={{ background: 'var(--type-animation)' }} />
        <i style={{ background: 'var(--type-texture)' }} />
        <i style={{ background: 'var(--type-material)' }} />
      </span>
      <span className="num">{t(dark ? 'inspector.calibrationDark' : 'inspector.calibration')}</span>
    </div>
  )
}

export interface Fact {
  label: string
  value: ReactNode
  title?: string
}

/** Wider values take two or three cells so they stay on one line. */
const spanOf = (v: ReactNode) => (typeof v === 'string' ? (v.length > 24 ? 3 : v.length > 11 ? 2 : 1) : 2)

/** The facts that decide, three to a row: the value in mono, its label below (the label comes first in the markup, as a definition list needs). */
export function FactsGrid({ facts }: { facts: Fact[] }) {
  if (!facts.length) return null
  const cells = facts.reduce((sum, f) => sum + spanOf(f.value), 0)
  // The last row is filled out, so the hairlines between cells run to the edge.
  const filler = (3 - (cells % 3)) % 3
  return (
    <dl className="facts">
      {facts.map((f, i) => (
        <div key={`${i}:${f.label}`} className="fact" style={{ gridColumn: `span ${spanOf(f.value)}` }} title={f.title}>
          <dt>{f.label}</dt>
          <dd>{f.value}</dd>
        </div>
      ))}
      {filler > 0 && <div className="fact fact-fill" style={{ gridColumn: `span ${filler}` }} aria-hidden="true" />}
    </dl>
  )
}

/** The measurements a summary holds, as facts (a catalog result has no more than this). */
export function summaryFacts(s: StatsSummary, t: Translate): Fact[] {
  const n = (v: number) => v.toLocaleString()
  const facts: Fact[] = []
  if (s.width) facts.push({ label: t('texture.size'), value: `${s.width} × ${s.height}` })
  if (s.duration !== undefined || s.clipLength !== undefined) facts.push({ label: t('audio.length'), value: formatDuration(s.duration ?? s.clipLength ?? 0) })
  if (s.triangles > 0) facts.push({ label: t('stats.triangles'), value: n(s.triangles) })
  if (s.vertices > 0) facts.push({ label: t('stats.vertices'), value: n(s.vertices) })
  if (s.particles !== undefined && s.triangles === 0) facts.push({ label: t('particles.peak'), value: `≈ ${n(s.particles)}` })
  if (s.glyphs !== undefined) facts.push({ label: t('font.glyphCount'), value: n(s.glyphs) })
  if (s.materials > 0) facts.push({ label: t('stats.materials'), value: n(s.materials) })
  if (s.bones > 0) facts.push({ label: t('stats.bones'), value: n(s.bones) })
  if (s.clips > 0) facts.push({ label: t('stats.clips'), value: n(s.clips) })
  if (s.fileBytes > 0) facts.push({ label: t('details.size'), value: (s.duration !== undefined ? '≈ ' : '') + formatBytes(s.fileBytes) })
  if (s.textureBytes > 0) facts.push({ label: t('stats.textures'), value: formatBytes(s.textureBytes) })
  return facts
}

/** Which preset a budget is: the two fixed ones are shared objects, anything else is custom. */
const presetOf = (budget: PlayableBudget) => (budget === PLAYABLE_BUDGET ? 'standard' : budget === TIGHT_BUDGET ? 'tight' : 'custom')

/** One line under the facts: it fits the budget, or what does not. The icon says it too, so colour is never alone. */
export function BudgetLine({ issues, budget }: { issues: BudgetIssue[]; budget: PlayableBudget }) {
  const { t } = useTranslation()
  if (!issues.length)
    return (
      <div className="budget" data-state="ok" role="status">
        <CheckIcon size={14} />
        <span>{t('inspector.fits', { preset: t(`inspector.presets.${presetOf(budget)}`) })}</span>
      </div>
    )
  return (
    <div className="budget" data-state="warn" role="status">
      <WarnIcon size={14} />
      <ul>
        {issues.map((i) => (
          <li key={i.key}>{issueText(t, i)}</li>
        ))}
      </ul>
    </div>
  )
}

const EDGE = 8
const GAP = 6

/**
 * Where a menu opens: by its button, kept inside the window (above or below as there is room).
 * The list sits on the window, so no panel with `overflow` cuts it off.
 */
export function useFloatingMenu(open: boolean, anchor: RefObject<HTMLElement>, align: 'start' | 'end', prefer: 'above' | 'below') {
  const list = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  useLayoutEffect(() => {
    if (!open) return setPos(null)
    const box = anchor.current?.getBoundingClientRect()
    const el = list.current
    if (!box || !el) return
    const w = el.offsetWidth
    const h = el.offsetHeight
    const left = Math.max(EDGE, Math.min(align === 'end' ? box.right - w : box.left, window.innerWidth - w - EDGE))
    const above = box.top - h - GAP
    const below = box.bottom + GAP
    const fitsAbove = above >= EDGE
    const fitsBelow = below + h <= window.innerHeight - EDGE
    const top = prefer === 'above' ? (fitsAbove ? above : fitsBelow ? below : EDGE) : fitsBelow ? below : fitsAbove ? above : EDGE
    setPos({ left, top: Math.max(EDGE, top) })
  }, [open, anchor, align, prefer])
  return { list, pos }
}

/** Closes a floating list on a press outside it, on resize or when the window loses focus. */
export function useMenuDismiss(open: boolean, close: () => void, ...inside: Array<RefObject<HTMLElement>>) {
  useEffect(() => {
    if (!open) return
    const outside = (e: PointerEvent) => !inside.some((r) => r.current?.contains(e.target as Node)) && close()
    const scrolled = (e: Event) => !inside.some((r) => r.current?.contains(e.target as Node)) && close()
    window.addEventListener('pointerdown', outside)
    window.addEventListener('scroll', scrolled, true)
    window.addEventListener('resize', close)
    window.addEventListener('blur', close)
    return () => {
      window.removeEventListener('pointerdown', outside)
      window.removeEventListener('scroll', scrolled, true)
      window.removeEventListener('resize', close)
      window.removeEventListener('blur', close)
    }
    // `inside` are refs: they do not change between renders.
  }, [open, close])
}

const MenuContext = createContext<() => void>(() => undefined)

/** A row of a menu; `checked` makes it a tick box. A pick closes the menu unless `keepOpen`. */
export function MenuItem({ icon, label, hint, checked, disabled, keepOpen, onSelect }: { icon?: ReactNode; label: ReactNode; hint?: ReactNode; checked?: boolean; disabled?: boolean; keepOpen?: boolean; onSelect(): void }) {
  const close = useContext(MenuContext)
  return (
    <button
      type="button"
      role={checked === undefined ? 'menuitem' : 'menuitemcheckbox'}
      aria-checked={checked}
      className="menu-pop-item"
      disabled={disabled}
      onClick={() => {
        onSelect()
        if (!keepOpen) close()
      }}
    >
      <span className="menu-pop-icon" aria-hidden="true">
        {checked !== undefined ? checked && <CheckIcon size={12} /> : icon}
      </span>
      <span className="menu-pop-label">{label}</span>
      {hint && <span className="num">{hint}</span>}
    </button>
  )
}

export const MenuLabel = ({ children }: { children: ReactNode }) => <div className="menu-label label-caps">{children}</div>
export const MenuSep = () => <div className="menu-sep" role="separator" />

/** "…": the actions that do not need a place in the row. Arrow keys move, Escape closes and gives focus back. */
export function MoreMenu({ label, children }: { label: string; children: ReactNode }) {
  const [open, setOpen] = useState(false)
  const box = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const close = useCallback(() => setOpen(false), [])
  const { list, pos } = useFloatingMenu(open, box, 'end', 'below')
  useMenuDismiss(open, close, box, list)
  // The first row takes focus once the list is placed, so the keyboard can go on from there.
  useEffect(() => {
    if (open && pos) list.current?.querySelector<HTMLElement>('[role^="menuitem"]:not(:disabled)')?.focus()
  }, [open, pos !== null])

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      setOpen(false)
      return trigger.current?.focus()
    }
    if (e.key === 'Tab') return setOpen(false)
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp' && e.key !== 'Home' && e.key !== 'End') return
    // A name being typed in the menu keeps its own cursor keys.
    if ((e.target as HTMLElement).tagName === 'INPUT') return
    e.preventDefault()
    const rows = [...(list.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]:not(:disabled)') ?? [])]
    if (!rows.length) return
    const at = rows.indexOf(document.activeElement as HTMLElement)
    const next = e.key === 'Home' ? 0 : e.key === 'End' ? rows.length - 1 : (at + (e.key === 'ArrowDown' ? 1 : -1) + rows.length) % rows.length
    rows[next].focus()
  }

  return (
    <div className="menu-anchor" ref={box}>
      <button ref={trigger} type="button" className="btn icon-btn" aria-haspopup="menu" aria-expanded={open} aria-label={label} title={label} onClick={() => setOpen(!open)}>
        <MoreIcon />
      </button>
      {open &&
        createPortal(
          <MenuContext.Provider value={close}>
            <div className="insp-menu" role="menu" aria-label={label} ref={list} onKeyDown={onKeyDown} style={pos ? { left: pos.left, top: pos.top } : { visibility: 'hidden' }}>
              {children}
            </div>
          </MenuContext.Provider>,
          document.body
        )}
    </div>
  )
}
