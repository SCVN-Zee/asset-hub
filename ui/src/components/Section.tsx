import { useState, type ReactNode } from 'react'
import { ChevronRightIcon } from './Icons'

const key = (id: string) => `assethub.section.${id}`
const stored = (id: string): boolean | undefined => {
  try {
    const v = localStorage.getItem(key(id))
    return v === null ? undefined : v === '1'
  } catch {
    return undefined
  }
}

/**
 * A details-panel section whose title opens and folds it; folded, a one-line
 * summary stands in for the content. Each person's choice is kept per section.
 */
export function Section({ id, title, summary, defaultOpen = true, children }: { id: string; title: ReactNode; summary?: ReactNode; defaultOpen?: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(() => stored(id) ?? defaultOpen)
  const toggle = () => {
    setOpen(!open)
    try {
      localStorage.setItem(key(id), open ? '0' : '1')
    } catch {
      // kept for this session only
    }
  }
  return (
    <section className="section" data-open={open || undefined}>
      <button type="button" className="section-head" aria-expanded={open} onClick={toggle}>
        <span className="section-chevron">
          <ChevronRightIcon size={12} />
        </span>
        <span className="label-caps">{title}</span>
        {!open && summary && <span className="section-summary">{summary}</span>}
      </button>
      {open && children}
    </section>
  )
}

/** A sidebar group (Watching, Recent, Collections, Folders) that folds under its title, remembered per group. */
export function SideGroup({ id, title, folded, defaultOpen = true, className, children }: { id: string; title: string; folded?: ReactNode; defaultOpen?: boolean; className?: string; children: ReactNode }) {
  const [open, setOpen] = useState(() => stored(`side.${id}`) ?? defaultOpen)
  const toggle = () => {
    setOpen(!open)
    try {
      localStorage.setItem(key(`side.${id}`), open ? '0' : '1')
    } catch {
      // kept for this session only
    }
  }
  return (
    <nav aria-label={title} className={`side-group ${className ?? ''}`} data-open={open || undefined}>
      <button type="button" className="side-head" aria-expanded={open} onClick={toggle}>
        <span className="section-chevron" data-open={open || undefined}>
          <ChevronRightIcon size={10} />
        </span>
        <span className="label-caps">{title}</span>
        {!open && folded !== undefined && <span className="muted small side-folded">{folded}</span>}
      </button>
      {open && children}
    </nav>
  )
}
