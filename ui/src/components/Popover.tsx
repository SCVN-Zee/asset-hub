import { useEffect, useRef, useState, type ReactNode } from 'react'

/** A button that opens a small panel below it; a click outside or Esc closes it. */
export function Popover({ trigger, label, className, children }: { trigger: ReactNode; label: string; className?: string; children: ReactNode }) {
  const [open, setOpen] = useState(false)
  const box = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const outside = (e: PointerEvent) => !box.current?.contains(e.target as Node) && setOpen(false)
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    window.addEventListener('pointerdown', outside)
    window.addEventListener('keydown', esc)
    return () => {
      window.removeEventListener('pointerdown', outside)
      window.removeEventListener('keydown', esc)
    }
  }, [open])
  return (
    <div className={`popover ${className ?? ''}`} ref={box}>
      <button type="button" className="btn btn-small popover-trigger" aria-haspopup="dialog" aria-expanded={open} aria-label={label} onClick={() => setOpen(!open)}>
        {trigger}
      </button>
      {open && (
        <div className="popover-panel" role="dialog" aria-label={label}>
          {children}
        </div>
      )}
    </div>
  )
}
