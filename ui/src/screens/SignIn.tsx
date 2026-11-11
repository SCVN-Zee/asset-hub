import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Language } from '@shared/settings'
import type { AuthStatus, InstallProgress } from '@shared/ipc'
import { ErrorNote } from '../components/ErrorNote'
import { LangToggle } from '../components/LangToggle'
import { VersionBadge } from '../components/VersionBadge'
import { RemoteError, unwrap } from '../remoteSource'

interface Props {
  language: Language
  onLanguage(language: Language): void
  onToken(token: string): Promise<AuthStatus>
  onGh(): Promise<AuthStatus>
  /** Signed in: show the hub. */
  onEnter(status: AuthStatus): void
}

type Busy = 'pat' | 'gh' | 'install' | undefined

/**
 * Sign-in: the app name, what it is for, and GitHub CLI as the way in. A
 * fine-grained token is the second way, one click away. The one-time code and
 * the install of gh show here too, as the steps happen.
 */
export function SignIn({ language, onLanguage, onToken, onGh, onEnter }: Props) {
  const { t } = useTranslation()
  const [token, setToken] = useState('')
  const [busy, setBusy] = useState<Busy>()
  const [error, setError] = useState<unknown>()
  const [ghPrompt, setGhPrompt] = useState<{ code: string; url: string }>()
  const [progress, setProgress] = useState<InstallProgress>()
  const [tokenOpen, setTokenOpen] = useState(false)
  const cancelled = useRef(false)

  useEffect(() => window.api.auth.onGhLoginPrompt(setGhPrompt), [])
  useEffect(() => window.api.tools.onInstallProgress(setProgress), [])

  const ghMissing = error instanceof RemoteError && error.error.code === 'ghNotFound'

  const run = async (kind: Exclude<Busy, undefined>, action: () => Promise<AuthStatus>) => {
    setBusy(kind)
    setError(undefined)
    cancelled.current = false
    try {
      onEnter(await action())
    } catch (err) {
      if (!cancelled.current) setError(err)
    } finally {
      setBusy(undefined)
      setGhPrompt(undefined)
      setProgress(undefined)
    }
  }

  /** Installs gh into the app folder, then continues straight into the gh sign-in. */
  const installThenSignIn = () =>
    run('install', async () => {
      unwrap(await window.api.tools.installGh())
      setProgress(undefined)
      setBusy('gh')
      return onGh()
    })

  const cancelGh = () => {
    cancelled.current = true
    void window.api.auth.cancelGhWebLogin()
  }

  const code = ghPrompt?.code.replace(/[^A-Z0-9]/gi, '').toUpperCase() ?? ''
  const percent = progress?.stage === 'download' && progress.total ? Math.round((progress.received / progress.total) * 100) : undefined

  return (
    <div className="signin">
      <div className="signin-top">
        <LangToggle value={language} onChange={onLanguage} />
      </div>

      <main className="signin-card">
        <h1 className="signin-title">
          <span className="brand-mark" aria-hidden="true" />
          Asset Hub
        </h1>
        <p className="signin-lead">{t('signin.tagline')}</p>

        {ghPrompt && busy === 'gh' ? (
          <section className="signin-panel" aria-live="polite">
            <span className="label-caps">{t('signin.code')}</span>
            <div className="signin-code" aria-label={ghPrompt.code}>
              {[...code].map((ch, i) => (
                <span key={i} className="signin-code-cell" data-gap={i === 4 || undefined}>
                  {ch}
                </span>
              ))}
            </div>
            <span className="small muted">{t('signin.ghCopied')}</span>
            <div className="signin-actions">
              <button type="button" className="btn btn-primary btn-lg" onClick={() => void window.api.openExternal(ghPrompt.url)}>
                {t('signin.ghOpen')}
              </button>
              <button type="button" className="btn btn-lg" onClick={cancelGh}>
                {t('signin.cancel')}
              </button>
            </div>
            <ol className="signin-steps">
              <li>
                <b>{t('signin.stepN', { n: 1 })}</b>
                {t('signin.stepShort1')}
              </li>
              <li>
                <b>{t('signin.stepN', { n: 2 })}</b>
                {t('signin.stepShort2')}
              </li>
              <li>
                <b>{t('signin.stepN', { n: 3 })}</b>
                {t('signin.stepShort3')}
              </li>
            </ol>
            <span className="small muted">{t('signin.ghWaiting')}</span>
          </section>
        ) : busy === 'install' ? (
          <section className="signin-panel" aria-live="polite">
            <div className="signin-install-head">
              <strong>{t('signin.installing')}</strong>
              {percent !== undefined && <span className="mono">{percent}%</span>}
            </div>
            <div className="signin-bar" role="progressbar" aria-label={t('signin.installing')} aria-valuenow={percent} aria-valuemin={0} aria-valuemax={100}>
              <span style={{ width: percent !== undefined ? `${percent}%` : '100%' }} data-indeterminate={percent === undefined || undefined} />
            </div>
            <span className="small muted">{progress ? t(`signin.installStage.${progress.stage}`) : t('signin.installStage.start')}</span>
          </section>
        ) : tokenOpen ? (
          <section className="signin-panel">
            <strong>{t('signin.tokenTitle')}</strong>
            <form
              className="signin-token"
              onSubmit={(e) => {
                e.preventDefault()
                void run('pat', () => onToken(token))
              }}
            >
              <label htmlFor="pat">{t('signin.patLabel')}</label>
              <div className="field-row">
                <input id="pat" className="input mono" type="password" placeholder="github_pat_…" autoComplete="off" autoFocus value={token} onChange={(e) => setToken(e.target.value)} />
                <button type="submit" className="btn btn-primary" disabled={busy !== undefined || !token.trim()}>
                  {busy === 'pat' ? t('signin.checking') : t('signin.enter')}
                </button>
              </div>
              <span className="small muted">{t('signin.patScope')}</span>
            </form>
            {error !== undefined && <ErrorNote error={error} />}
            <button type="button" className="inline-link signin-back" onClick={() => setTokenOpen(false)}>
              ‹ {t('signin.back')}
            </button>
          </section>
        ) : ghMissing && !busy ? (
          <section className="signin-panel">
            <strong>{t('signin.ghMissingTitle')}</strong>
            <div className="signin-actions">
              <button type="button" className="btn btn-primary btn-lg" onClick={() => void installThenSignIn()}>
                {t('signin.installGh')}
              </button>
            </div>
          </section>
        ) : (
          <div className="signin-start">
            {error !== undefined && <ErrorNote error={error} />}
            <button type="button" className="btn btn-primary btn-lg signin-primary" disabled={busy !== undefined} aria-busy={busy === 'gh'} onClick={() => void run('gh', onGh)}>
              {busy === 'gh' ? t('signin.connecting') : error !== undefined ? t('signin.retry') : t('signin.start')}
            </button>
            <button type="button" className="inline-link" disabled={busy !== undefined} onClick={() => setTokenOpen(true)}>
              {t('signin.tokenLink')}
            </button>
          </div>
        )}
      </main>

      <footer className="signin-foot">
        <VersionBadge />
      </footer>
    </div>
  )
}
