import { useTranslation } from 'react-i18next'
import { RemoteError } from '../remoteSource'

interface Props {
  error: unknown
}

/** Translated error with the raw message underneath, plus a button for SSO authorisation. */
export function ErrorNote({ error }: Props) {
  const { t } = useTranslation()
  const info = error instanceof RemoteError ? error.error : undefined
  const code = info?.code ?? 'unknown'
  const raw = error instanceof Error ? error.message : String(error)
  const time = info?.resetAt ? new Date(info.resetAt).toLocaleTimeString() : ''
  return (
    <div className="error-note" role="alert">
      <strong>{t(`errors.${code}`, { time })}</strong>
      {raw && <span className="mono small muted">{raw}</span>}
      {info?.ssoUrl && (
        <button type="button" className="btn" onClick={() => void window.api.openExternal(info.ssoUrl!)}>
          {t('errors.openSso')}
        </button>
      )}
    </div>
  )
}
