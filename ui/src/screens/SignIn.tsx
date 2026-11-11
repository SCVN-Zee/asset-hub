import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { LANGUAGES, type Language } from '@shared/settings'
import type { AuthStatus, InstallProgress } from '@shared/ipc'
import { ErrorNote } from '../components/ErrorNote'
import { RemoteError, unwrap } from '../remoteSource'
import type { LobbyScene } from '../viewer/lobbyScene'
import { VersionBadge } from '../components/VersionBadge'
import { isMuted, playChestOpen, playClick, setMuted } from '../lobbySound'

interface Props {
  language: Language
  onLanguage(language: Language): void
  onToken(token: string): Promise<AuthStatus>
  onGh(): Promise<AuthStatus>
  /** Signed in and the chest has opened: show the hub. */
  onEnter(status: AuthStatus): void
  /** Already signed in (the app was opened again): the chest opens by itself, no start button. */
  autoEnter?: AuthStatus
  /** The previous sign-in is still being checked: no sign-in buttons yet. */
  checking?: boolean
}

type Busy = 'pat' | 'gh' | 'install' | undefined

/** Without the 3D scene (no WebGL), a short white-out stands in for the opening. */
const OPEN_MS_FALLBACK = 500

/**
 * Sign-in as a game lobby: the chest, one big start button, the one-time code
 * as a room code, installing gh as a loading bar, a token as the way round.
 * Signing in opens the chest; the camera dives into its light and the hub
 * takes over (`onEnter`).
 */
export function SignIn({ language, onLanguage, onToken, onGh, onEnter, autoEnter, checking }: Props) {
  const { t } = useTranslation()
  const [token, setToken] = useState('')
  const [busy, setBusy] = useState<Busy>()
  const [error, setError] = useState<unknown>()
  const [ghPrompt, setGhPrompt] = useState<{ code: string; url: string }>()
  const [progress, setProgress] = useState<InstallProgress>()
  const [tokenOpen, setTokenOpen] = useState(false)
  const [opening, setOpening] = useState(false)
  const [muted, setMutedState] = useState(isMuted)
  const canvas = useRef<HTMLCanvasElement>(null)
  /** The 3D lobby once loaded; `null` when it cannot run (no WebGL), `undefined` while loading. */
  const scene = useRef<LobbyScene | null>()
  const chestSpace = useRef<HTMLDivElement>(null)
  const [sceneState, setSceneState] = useState<'loading' | 'ready' | 'none'>('loading')
  const entered = useRef(false)
  const cancelled = useRef(false)

  // three.js loads with this screen only, not with the app.
  useEffect(() => {
    let disposed = false
    void import('../viewer/lobbyScene')
      .then(({ LobbyScene }) => {
        if (disposed || !canvas.current) return
        scene.current = new LobbyScene(canvas.current)
        fitChest()
        setSceneState('ready')
      })
      .catch(() => {
        scene.current = null
        if (!disposed) setSceneState('none')
      })
    // The chest goes where the page leaves room for it, whatever the window size.
    const fitChest = () => {
      const space = chestSpace.current?.getBoundingClientRect()
      const box = canvas.current?.getBoundingClientRect()
      if (space && box && scene.current) scene.current.setSlot(space.top - box.top + space.height / 2, space.height)
    }
    const watch = new ResizeObserver(fitChest)
    if (chestSpace.current) watch.observe(chestSpace.current)
    if (canvas.current) watch.observe(canvas.current)
    return () => {
      disposed = true
      watch.disconnect()
      scene.current?.dispose()
    }
  }, [])

  useEffect(() => window.api.auth.onGhLoginPrompt(setGhPrompt), [])
  useEffect(() => window.api.tools.onInstallProgress(setProgress), [])

  const ghMissing = error instanceof RemoteError && error.error.code === 'ghNotFound'

  /** The chest opens, then the hub is shown once the light has filled the screen (sooner when skipped). */
  const open = (status: AuthStatus) => {
    setTokenOpen(false)
    setOpening(true)
    playChestOpen()
    // A click or any key skips the rest.
    const enter = () => {
      window.removeEventListener('pointerdown', enter)
      window.removeEventListener('keydown', enter)
      if (entered.current) return
      entered.current = true
      onEnter(status)
    }
    void (scene.current ? scene.current.open() : new Promise((r) => window.setTimeout(r, OPEN_MS_FALLBACK))).then(enter)
    window.setTimeout(() => {
      if (entered.current) return
      window.addEventListener('pointerdown', enter)
      window.addEventListener('keydown', enter)
    }, 200)
  }

  // Opened again while signed in: the chest opens as soon as the scene is there.
  useEffect(() => {
    if (autoEnter && sceneState !== 'loading' && !opening) open(autoEnter)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoEnter, sceneState])

  const run = async (kind: Exclude<Busy, undefined>, action: () => Promise<AuthStatus>) => {
    setBusy(kind)
    setError(undefined)
    cancelled.current = false
    try {
      open(await action())
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
  const idle = !busy && !opening && !autoEnter && !checking

  return (
    <div className="lobby" data-opening={opening || undefined} data-scene={sceneState}>
      <canvas ref={canvas} className="lobby-canvas" aria-hidden="true" />

      <main className="lobby-center">
          {/* The 3D chest sits here, drawn by the scene behind. */}
          <div className="lobby-chest-space" ref={chestSpace} aria-hidden="true" />
          <h1 className="lobby-title">Asset Hub</h1>

          {!autoEnter && !checking && (
          <div className="lobby-stage">
            {ghPrompt && busy === 'gh' ? (
              <section className="lobby-panel" aria-live="polite">
                <span className="lobby-caps">{t('signin.roomCode')}</span>
                <div className="lobby-code" aria-label={ghPrompt.code}>
                  {[...code].map((ch, i) => (
                    <span key={i} className="lobby-code-cell" data-gap={i === 4 || undefined}>
                      {ch}
                    </span>
                  ))}
                </div>
                <span className="small lobby-copied">{t('signin.ghCopied')}</span>
                <div className="lobby-row">
                  <button type="button" className="lobby-btn lobby-btn-primary" onClick={() => void window.api.openExternal(ghPrompt.url)}>
                    {t('signin.ghOpen')}
                  </button>
                  <button type="button" className="lobby-btn" onClick={cancelGh}>
                    {t('signin.cancel')}
                  </button>
                </div>
                <ol className="lobby-steps">
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
                <span className="small muted lobby-blink">{t('signin.ghWaiting')}</span>
              </section>
            ) : busy === 'install' ? (
              <section className="lobby-loading" aria-live="polite">
                <div className="lobby-loading-head">
                  <span className="lobby-caps lobby-caps-strong">{t('signin.installing')}</span>
                  {percent !== undefined && <span className="lobby-percent">{percent}%</span>}
                </div>
                <div className="lobby-bar" role="progressbar" aria-label={t('signin.installing')} aria-valuenow={percent} aria-valuemin={0} aria-valuemax={100}>
                  <span style={{ width: percent !== undefined ? `${percent}%` : '100%' }} data-indeterminate={percent === undefined || undefined} />
                </div>
                <span className="small muted">{progress ? t(`signin.installStage.${progress.stage}`) : t('signin.installStage.start')}</span>
              </section>
            ) : tokenOpen && !opening ? (
              <section className="lobby-panel lobby-panel-left">
                <span className="lobby-caps">{t('signin.tokenTitle')}</span>
                <form
                  className="lobby-token"
                  onSubmit={(e) => {
                    e.preventDefault()
                    void run('pat', () => onToken(token))
                  }}
                >
                  <label htmlFor="pat">{t('signin.patLabel')}</label>
                  <div className="lobby-row">
                    <input id="pat" className="input mono" type="password" placeholder="github_pat_…" autoComplete="off" autoFocus value={token} onChange={(e) => setToken(e.target.value)} />
                    <button type="submit" className="lobby-btn lobby-btn-primary" disabled={busy !== undefined || !token.trim()}>
                      {busy === 'pat' ? t('signin.checking') : t('signin.enter')}
                    </button>
                  </div>
                  <span className="small muted">{t('signin.patScope')}</span>
                </form>
                {error !== undefined && <ErrorNote error={error} />}
                <button type="button" className="lobby-link" onClick={() => setTokenOpen(false)}>
                  ‹ {t('signin.back')}
                </button>
              </section>
            ) : (
              <div className="lobby-start-wrap">
                {ghMissing && !busy ? (
                  <section className="lobby-panel">
                    <strong>{t('signin.ghMissingTitle')}</strong>
                    <button type="button" className="lobby-btn lobby-btn-primary" onClick={() => void installThenSignIn()}>
                      {t('signin.installGh')}
                    </button>
                  </section>
                ) : (
                  <>
                    {error !== undefined && (
                      <div className="lobby-error">
                        <ErrorNote error={error} />
                      </div>
                    )}
                    <button
                      type="button"
                      className="lobby-start"
                      disabled={!idle}
                      aria-busy={busy === 'gh'}
                      // The chest answers the button: it rattles and glints while START is under the pointer or focused.
                      onPointerEnter={() => scene.current?.setHover(true)}
                      onPointerLeave={() => scene.current?.setHover(false)}
                      onFocus={() => scene.current?.setHover(true)}
                      onBlur={() => scene.current?.setHover(false)}
                      onClick={() => {
                        playClick()
                        scene.current?.setHover(false)
                        void run('gh', onGh)
                      }}
                    >
                      <svg width="26" height="26" viewBox="0 0 24 24" aria-hidden="true">
                        <path d="M7 4.5v15l12.5-7.5z" fill="currentColor" />
                      </svg>
                      {busy === 'gh' ? t('signin.connecting') : error !== undefined ? t('signin.retry') : t('signin.start')}
                    </button>
                  </>
                )}
              </div>
            )}
          </div>
          )}
      </main>

      <div className="lobby-top">
        <button
          type="button"
          className="lobby-sound"
          aria-pressed={!muted}
          aria-label={t('signin.sound')}
          title={muted ? t('signin.soundOff') : t('signin.soundOn')}
          onClick={() => {
            setMuted(!muted)
            setMutedState(!muted)
            if (muted) playClick()
          }}
        >
          <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M4 9v6h4l5 4V5L8 9z" fill="currentColor" />
            {muted ? <path d="M17 9l5 6M22 9l-5 6" /> : <path d="M16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12" />}
          </svg>
        </button>
        <div className="lobby-langs" role="group" aria-label={t('common.language')}>
          {LANGUAGES.map((l) => (
            <button key={l} type="button" className="lobby-lang" aria-pressed={language === l} onClick={() => onLanguage(l)}>
              {l.toUpperCase()}
            </button>
          ))}
        </div>
      </div>

      <footer className="lobby-bottom">
        {idle && !tokenOpen && (
          <button type="button" className="lobby-link" onClick={() => setTokenOpen(true)}>
            {t('signin.tokenLink')}
          </button>
        )}
        <span className="lobby-sign">
          <span className="lobby-credit">made by Dan</span>
          <span className="lobby-version">
            <VersionBadge />
          </span>
        </span>
      </footer>

      <div className="lobby-flash" aria-hidden="true" />
    </div>
  )
}
