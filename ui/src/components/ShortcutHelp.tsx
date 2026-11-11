import { useTranslation } from 'react-i18next'
import { Dialog } from './Dialog'

/** The hub's keyboard shortcuts, as keys and what they do (`?` shows this). */
const SHORTCUTS: Array<[string, string]> = [
  ['⌘ K', 'search'],
  ['← → ↑ ↓', 'move'],
  ['⌘ / ⇧ click', 'pick'],
  ['⌘ A', 'pickAll'],
  ['Esc', 'clear'],
  ['F', 'favourite'],
  ['E', 'exportList'],
  ['⌘ I', 'unity'],
  ['?', 'help']
]

export function ShortcutHelp({ onClose }: { onClose(): void }) {
  const { t } = useTranslation()
  return (
    <Dialog title={t('shortcuts.title')} onClose={onClose} closeKey="?" className="dialog-narrow">
      <dl className="shortcut-list">
        {SHORTCUTS.map(([keys, what]) => (
          <div key={what}>
            <dt>
              <kbd>{keys}</kbd>
            </dt>
            <dd>{t(`shortcuts.${what}`)}</dd>
          </div>
        ))}
      </dl>
    </Dialog>
  )
}
