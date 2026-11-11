import { useTranslation } from 'react-i18next'
import { RemoteError } from '../remoteSource'
import { WarnIcon } from './Icons'

interface Props {
  error: unknown
  /** Extra classes, e.g. the gutter it needs inside the content area. */
  className?: string
}

/** Translated error with the raw message underneath, plus a button for SSO authorisation. */
export function ErrorNote({ error, className }: Props) {
  const { t } = useTranslation()
  const info = error instanceof RemoteError ? error.error : undefined
  const code = info?.code ?? 'unknown'
  const raw = error instanceof Error ? error.message : String(error)
  const time = info?.resetAt ? new Date(info.resetAt).toLocaleTimeString() : ''
  return (
    <div className={className ? `error-note ${className}` : 'error-note'} role="alert">
      <strong className="error-title">
        <WarnIcon />
        <span>{t(`errors.${code}`, { time })}</span>
      </strong>
      {raw && <span className="mono small muted">{raw}</span>}
      {info?.ssoUrl && (
        <button type="button" className="btn" onClick={() => void window.api.openExternal(info.ssoUrl!)}>
          {t('errors.openSso')}
        </button>
      )}
    </div>
  )
}
