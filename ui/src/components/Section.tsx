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
 * A details-panel section whose head opens and folds it: chevron, title and, folded, a
 * one-line summary under the title. Each person's choice is kept per section.
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
    <section className="fold" data-open={open || undefined}>
      <button type="button" className="fold-head" aria-expanded={open} onClick={toggle}>
        <span className="section-chevron" data-open={open || undefined}>
          <ChevronRightIcon size={12} />
        </span>
        <span className="fold-title">{title}</span>
        {!open && summary && <span className="fold-summary">{summary}</span>}
      </button>
      {open && <div className="fold-body">{children}</div>}
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
