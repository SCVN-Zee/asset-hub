import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'

/** The hub's keyboard shortcuts, as keys and what they do (`?` shows this). */
const SHORTCUTS: Array<[string, string]> = [
  ['⌘ K', 'search'],
  ['← → ↑ ↓', 'move'],
  ['⌘ / ⇧ click', 'pick'],
  ['⌘ A', 'pickAll'],
  ['Esc', 'clear'],
  ['F', 'favourite'],
  ['E', 'chest'],
  ['⌘ I', 'unity'],
  ['⌘ J', 'pet'],
  ['?', 'help']
]

export function ShortcutHelp({ onClose }: { onClose(): void }) {
  const { t } = useTranslation()
  useEffect(() => {
    const close = (e: KeyboardEvent) => (e.key === 'Escape' || e.key === '?') && (e.preventDefault(), onClose())
    window.addEventListener('keydown', close)
    return () => window.removeEventListener('keydown', close)
  }, [onClose])
  return (
    <div className="shortcut-backdrop" onClick={onClose}>
      <div className="shortcut-card" role="dialog" aria-label={t('shortcuts.title')} onClick={(e) => e.stopPropagation()}>
        <strong>{t('shortcuts.title')}</strong>
        <dl>
          {SHORTCUTS.map(([keys, what]) => (
            <div key={what}>
              <dt>
                <kbd>{keys}</kbd>
              </dt>
              <dd>{t(`shortcuts.${what}`)}</dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  )
}
