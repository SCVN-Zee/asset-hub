import { useEffect, useRef, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { CloseIcon } from './Icons'

interface Props {
  title: string
  onClose(): void
  /** A second key that closes it (the shortcut list closes on `?` too). */
  closeKey?: string
  /** Width class, e.g. `dialog-narrow`. */
  className?: string
  children: ReactNode
}

/**
 * A modal over the hub in the settings panel's style: graphite card, heading, close button.
 * Focus moves into it, Escape closes it, and focus goes back to where it came from.
 */
export function Dialog({ title, onClose, closeKey, className, children }: Props) {
  const { t } = useTranslation()
  const card = useRef<HTMLDivElement>(null)
  // Read while rendering, before an autofocused field takes focus.
  const before = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : undefined)
  const close = useRef(onClose)
  close.current = onClose

  useEffect(() => {
    if (!card.current?.contains(document.activeElement)) card.current?.focus()
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return
      if (e.key === 'Escape' || (closeKey && e.key === closeKey)) {
        e.preventDefault()
        close.current()
      }
    }
    window.addEventListener('keydown', onKey)
    const opener = before.current
    return () => {
      window.removeEventListener('keydown', onKey)
      if (opener?.isConnected) opener.focus()
    }
  }, [closeKey])

  return (
    <div
      className="dialog-layer"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div className={className ? `dialog-card ${className}` : 'dialog-card'} role="dialog" aria-modal="true" aria-label={title} tabIndex={-1} ref={card}>
        <div className="dialog-head">
          <h1>{title}</h1>
          <button type="button" className="btn icon-btn" aria-label={t('common.close')} title={t('common.close')} onClick={onClose}>
            <CloseIcon />
          </button>
        </div>
        <div className="dialog-body">{children}</div>
      </div>
    </div>
  )
}
