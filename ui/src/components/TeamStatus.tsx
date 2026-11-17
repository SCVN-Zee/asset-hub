import { useEffect, useState, type CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'
import type { TeamStatus as Status } from '@shared/ipc'

// A quiet dot: it adds no words to the top bar and shows only while the team backend is out of reach.
const dot: CSSProperties = { flex: 'none', width: 6, height: 6, borderRadius: '50%', background: 'var(--mute)', opacity: 0.8 }

/**
 * Shows a small muted dot while the team backend is offline. Browsing and previews work either
 * way, so `online` and `disabled` (signed out, or the client is off) show nothing.
 */
export function TeamStatus() {
  const { t } = useTranslation()
  const [status, setStatus] = useState<Status>('online')

  useEffect(() => {
    let changed = false
    const stop = window.api.team.onStatus((next) => {
      changed = true
      setStatus(next)
    })
    // The answer of the first look must not overwrite a change that arrived while it was on the way.
    void window.api.team.status().then(
      (first) => {
        if (!changed) setStatus(first)
      },
      () => undefined
    )
    return stop
  }, [])

  if (status !== 'offline') return null
  return <span role="img" style={dot} title={t('team.offlineHint')} aria-label={t('team.offline')} />
}
