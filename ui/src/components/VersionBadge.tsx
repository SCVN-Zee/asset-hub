import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { InstallProgress, UpdateInfo } from '@shared/ipc'

/**
 * The app's version beside its name and, when the release repo has a later
 * one (asked once per launch), a button that installs it and restarts the app,
 * with its progress. Should that fail, the button tries again.
 */
export function VersionBadge() {
  const { t } = useTranslation()
  const [info, setInfo] = useState<UpdateInfo | null>(null)
  const [busy, setBusy] = useState(false)
  const [progress, setProgress] = useState<InstallProgress>()
  const [failure, setFailure] = useState<string>()

  useEffect(() => {
    void window.api.update.check().then(setInfo)
  }, [])
  // Progress of an install, started by the app at launch or by the button.
  useEffect(
    () =>
      window.api.update.onProgress((p) => {
        if (p.stage === 'failed') {
          setFailure(p.message)
          setBusy(false)
          setProgress(undefined)
          return
        }
        setBusy(true)
        setProgress(p)
      }),
    []
  )

  const latest = info?.latest
  const install = async () => {
    if (!info || !latest) return
    // Not replaceable from here (e.g. run from the .dmg): the release page, for the installer.
    if (!info.canInstall) return void window.api.openExternal(info.page)
    setBusy(true)
    setFailure(undefined)
    const result = await window.api.update.install(latest)
    // On success the app quits and the new version opens.
    if (!result.ok) {
      setFailure(result.message)
      setBusy(false)
      setProgress(undefined)
    }
  }

  const percent = progress?.stage === 'download' && progress.total ? Math.round((progress.received / progress.total) * 100) : undefined
  const label = busy
    ? progress?.stage === 'verify'
      ? t('update.verify')
      : progress?.stage === 'extract'
        ? t('update.extract')
        : t('update.downloading', { percent: percent ?? 0 })
    : failure
      ? t('update.retry')
      : info?.canInstall
        ? t('update.install', { version: latest })
        : t('update.download', { version: latest })

  return (
    <>
      <span className="brand-version">v{__APP_VERSION__}</span>
      {latest && (
        <button
          type="button"
          className="update-pill"
          data-failed={failure ? true : undefined}
          disabled={busy}
          title={failure ? t('update.failed', { message: failure }) : t('update.hint', { version: latest })}
          onClick={() => void install()}
        >
          {busy && percent !== undefined && <span className="update-pill-fill" style={{ width: `${percent}%` }} />}
          <span className="update-pill-label">{label}</span>
        </button>
      )}
    </>
  )
}
