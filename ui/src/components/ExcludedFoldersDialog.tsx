import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Dialog } from './Dialog'

interface Props {
  /** The saved patterns, one per line. */
  patterns: string[]
  onSave(patterns: string[]): void
  onClose(): void
}

/** The excluded-folder patterns as text, one per line; Cancel drops the edits. */
export function ExcludedFoldersDialog({ patterns, onSave, onClose }: Props) {
  const { t } = useTranslation()
  const [text, setText] = useState(patterns.join('\n'))

  return (
    <Dialog title={t('settings.excluded')} onClose={onClose} className="dialog-wide">
      <textarea
        className="input mono excluded-editor"
        autoFocus
        spellCheck={false}
        value={text}
        rows={12}
        placeholder={'_ThirdParty\nAssets/Plugins/\n**/Samples~/'}
        onChange={(e) => setText(e.target.value)}
      />
      <span className="muted small">{t('settings.excludedHint')}</span>
      <div className="setting-actions">
        <button type="button" className="btn" onClick={onClose}>
          {t('settings.excludedCancel')}
        </button>
        <button type="button" className="btn btn-primary" onClick={() => onSave(text.split('\n'))}>
          {t('settings.excludedSave')}
        </button>
      </div>
    </Dialog>
  )
}
