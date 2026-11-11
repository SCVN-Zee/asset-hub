import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { FEEDBACK_KINDS, type FeedbackKind, type FeedbackReport, type FeedbackResult } from '@shared/usage'

interface Props {
  /** Where the user is, offered to go with the report. */
  context?: FeedbackReport['context']
  onClose(): void
}

/** Bug reports and ideas, opened as issues in the team's feedback repo. */
export function FeedbackDialog({ context, onClose }: Props) {
  const { t } = useTranslation()
  const [kind, setKind] = useState<FeedbackKind>('bug')
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [withContext, setWithContext] = useState(true)
  const [state, setState] = useState<{ status: 'idle' | 'sending' } | { status: 'done'; result: FeedbackResult }>({ status: 'idle' })

  useEffect(() => {
    const close = (e: KeyboardEvent) => e.key === 'Escape' && (e.preventDefault(), onClose())
    window.addEventListener('keydown', close)
    return () => window.removeEventListener('keydown', close)
  }, [onClose])

  const send = async () => {
    setState({ status: 'sending' })
    const result = await window.api.feedback.send({ kind, title, body, ...(withContext && context ? { context } : {}) })
    setState({ status: 'done', result })
  }

  const where = context && [context.repo, context.branch && `@ ${context.branch}`, context.asset].filter(Boolean).join(' ')

  return (
    <div className="shortcut-backdrop" onClick={onClose}>
      <div className="shortcut-card feedback-card" role="dialog" aria-label={t('feedback.title')} onClick={(e) => e.stopPropagation()}>
        <strong>{t('feedback.title')}</strong>
        {state.status === 'done' ? (
          <>
            <span>{state.result.sent ? t('feedback.sent') : t('feedback.queued')}</span>
            <div className="setting-actions">
              {state.result.sent && (
                <button type="button" className="btn" onClick={() => state.result.sent && void window.api.openExternal(state.result.url)}>
                  {t('feedback.open')}
                </button>
              )}
              <button type="button" className="btn btn-primary" onClick={onClose}>
                {t('feedback.close')}
              </button>
            </div>
          </>
        ) : (
          <>
            <div className="feedback-kinds" role="group" aria-label={t('feedback.kind')}>
              {FEEDBACK_KINDS.map((k) => (
                <button key={k} type="button" className="chip" aria-pressed={kind === k} onClick={() => setKind(k)}>
                  {t(`feedback.kinds.${k}`)}
                </button>
              ))}
            </div>
            <input className="input" autoFocus value={title} maxLength={120} placeholder={t('feedback.titlePlaceholder')} onChange={(e) => setTitle(e.target.value)} />
            <textarea className="input feedback-body" value={body} rows={6} placeholder={t(`feedback.bodyPlaceholder.${kind}`)} onChange={(e) => setBody(e.target.value)} />
            {where && (
              <label className="check small feedback-where">
                <input type="checkbox" checked={withContext} onChange={(e) => setWithContext(e.target.checked)} />
                <span>{t('feedback.withContext', { where })}</span>
              </label>
            )}
            <span className="muted small">{t('feedback.note')}</span>
            <div className="setting-actions">
              <button type="button" className="btn" onClick={onClose}>
                {t('feedback.cancel')}
              </button>
              <button type="button" className="btn btn-primary" disabled={!title.trim() || state.status === 'sending'} onClick={() => void send()}>
                {state.status === 'sending' ? t('feedback.sending') : t('feedback.send')}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
