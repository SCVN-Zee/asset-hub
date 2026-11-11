import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { DependencyReport, ModelStats } from '@shared/types'
import { ViewGizmo } from './ViewGizmo'
import { FbxViewer } from '../viewer/FbxViewer'
import { buildPrefab, dressPrefab, type PrefabBuild } from '../viewer/prefabBuild'
import { buildEffect } from '../viewer/effectBuild'
import { FRAME, type EffectPlayer } from '../viewer/particlePlayer'
import type { LiveContext } from './ModelViewer'

interface Props {
  live: LiveContext
  /** Repo path of the prefab. */
  path: string
  report?: DependencyReport
  onStats(stats: ModelStats): void
  onThumb(dataUrl: string): void
  onLoading?(loading: boolean): void
  /** What the viewer sits on now, so the calibration strip can say so. */
  onBackdrop?(backdrop: FxBackdrop): void
}

/** An effect is judged on the stage grey or on dark, where glow and additive particles read as in a game. */
export type FxBackdrop = 'stage' | 'dark'
const BACKDROP_KEY = 'assethub.fxBackdrop'

/** Remembered per machine; effects start on dark, and storage can be unavailable (then it is dark too). */
function readBackdrop(): FxBackdrop {
  try {
    return localStorage.getItem(BACKDROP_KEY) === 'stage' ? 'stage' : 'dark'
  } catch {
    return 'dark'
  }
}

function writeBackdrop(value: FxBackdrop): void {
  try {
    localStorage.setItem(BACKDROP_KEY, value)
  } catch {
    // Not remembered: still applies until the app closes.
  }
}

type Status = 'waiting' | 'loading' | 'ready' | 'empty' | 'error'

/** What a build depends on: rebuilt only when the report finds more files, not on every step. */
const buildKey = (r: DependencyReport) => [...Object.keys(r.guidPaths ?? {}).sort(), ...r.materials.flatMap((m) => m.textures.map((t) => t.path ?? ''))].join('|')

/** A prefab put together from its models and materials, with the model viewer's controls. */
export function PrefabViewer({ live, path, report, onStats, onThumb, onLoading, onBackdrop }: Props) {
  const { t } = useTranslation()
  const host = useRef<HTMLDivElement>(null)
  const viewer = useRef<FbxViewer>()
  const built = useRef<{ key: string; build: PrefabBuild; effect?: EffectPlayer; stop?: () => void }>()
  const [status, setStatus] = useState<Status>('waiting')
  const [error, setError] = useState('')
  const [missing, setMissing] = useState(0)
  const [autoRotate, setAutoRotate] = useState(false)
  const [wireframe, setWireframe] = useState(false)
  /** Particle controls, shown when the prefab has particle systems. */
  const [fx, setFx] = useState<{ paused: boolean; loop: boolean; speed: number }>()
  /** Where the effect is on its timeline, in seconds, and how long that runs. */
  const [clock, setClock] = useState({ time: 0, length: 1 })
  const [backdrop, setBackdrop] = useState(readBackdrop)
  // Only effects have the choice; a model always sits on the stage.
  const shownBackdrop: FxBackdrop = fx ? backdrop : 'stage'
  const callbacks = useRef({ onStats, onThumb, onLoading, onBackdrop })
  callbacks.current = { onStats, onThumb, onLoading, onBackdrop }

  useEffect(() => {
    callbacks.current.onBackdrop?.(shownBackdrop)
  }, [shownBackdrop])
  useEffect(() => () => callbacks.current.onBackdrop?.('stage'), [])

  useEffect(() => {
    callbacks.current.onLoading?.(status === 'loading' || status === 'waiting')
  }, [status])
  useEffect(() => () => callbacks.current.onLoading?.(false), [])

  useEffect(() => {
    const el = host.current
    if (!el) return
    const v = new FbxViewer(el)
    viewer.current = v
    setStatus('waiting')
    setMissing(0)
    setFx(undefined)
    setAutoRotate(false)
    setWireframe(false)
    return () => {
      built.current?.stop?.()
      built.current?.effect?.dispose()
      v.dispose()
      viewer.current = undefined
      built.current?.build.dispose()
      built.current = undefined
    }
  }, [live.repo, live.branch, live.root, path])

  useEffect(() => {
    if (!report) return
    const key = buildKey(report)
    if (built.current?.key === key) return
    const owner = viewer.current
    let cancelled = false
    if (!built.current) setStatus('loading')
    void (async () => {
      try {
        const build = await buildPrefab(live, path, report)
        const effect = build.anchors.length ? await buildEffect(live, report, build.anchors) : undefined
        if (cancelled || !owner || owner.isDisposed() || owner !== viewer.current) {
          effect?.dispose()
          return build.dispose()
        }
        const previous = built.current
        previous?.stop?.()
        previous?.effect?.dispose()
        let shown = -1
        built.current = {
          key,
          build,
          effect,
          stop:
            effect &&
            owner.addUpdater((dt) => {
              effect.update(dt)
              // The timeline follows the play, one render per frame of it at most.
              const at = Math.floor(effect.time / FRAME)
              if (at !== shown) {
                shown = at
                setClock({ time: effect.time, length: effect.timeline() })
              }
            })
        }
        // The camera takes in where particles fly, not only the meshes.
        owner.showGroup(build.group, effect?.measuredExtent())
        await dressPrefab(owner, build)
        previous?.build.dispose()
        if (cancelled) return
        setMissing(build.missing)
        if (effect) {
          owner.setWireframe(wireframe)
          // A rebuild (more files found) keeps the controls as the user left them.
          setFx((f) => {
            const next = f ?? { paused: false, loop: true, speed: 1 }
            Object.assign(effect, next)
            return next
          })
          // It played while hidden by the overlay (textures were still loading): from the start, now that it shows.
          effect.restart()
          setStatus('ready')
          // No snapshot: effect cards are filmed through their whole run in the background.
          callbacks.current.onStats(owner.stats(build.modelBytes, build.textures))
          return
        }
        // Nothing found yet while models are still being looked up: keep waiting for the next step.
        if (!build.meshes) return setStatus(report.partial ? 'loading' : 'empty')
        owner.setWireframe(wireframe)
        setStatus('ready')
        callbacks.current.onStats(owner.stats(build.modelBytes, build.textures))
        setTimeout(() => !owner.isDisposed() && callbacks.current.onThumb(owner.snapshot()), 300)
      } catch (err) {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
        setStatus('error')
      }
    })()
    return () => {
      cancelled = true
    }
  }, [report])

  const setEffect = (patch: Partial<NonNullable<typeof fx>>) => {
    const next = { paused: false, loop: true, speed: 1, ...fx, ...patch }
    const effect = built.current?.effect
    if (effect) Object.assign(effect, next)
    setFx(next)
  }
  const overlay =
    status === 'waiting' || status === 'loading'
      ? t('prefabView.loading')
      : status === 'empty'
        ? t(report?.warnings.includes('uiPrefab') ? 'prefabView.ui' : report?.warnings.includes('particles') ? 'prefabView.particles' : 'prefabView.empty')
        : status === 'error'
          ? t('viewer.error', { message: error })
          : undefined

  /** Holds the effect at `seconds` into its play (a frame of the timeline). */
  const seek = (seconds: number) => {
    const effect = built.current?.effect
    if (!effect || !fx) return
    if (!fx.paused) setEffect({ paused: true })
    const length = effect.timeline()
    effect.seek(Math.min(length, Math.max(0, Math.round(seconds / FRAME) * FRAME)))
    setClock({ time: effect.time, length })
  }
  const frameNow = Math.round(clock.time / FRAME)

  /** Another play of the effect on top of the running ones, like spawning the prefab again (spam it like in Unity). */
  const fire = () => {
    const effect = built.current?.effect
    if (!effect || !fx) return
    if (fx.paused) setEffect({ paused: false })
    effect.fire()
  }

  return (
    <div
      className="viewer"
      data-backdrop={shownBackdrop}
      // Focused (clicked) viewer: Space fires the effect again.
      tabIndex={fx ? 0 : undefined}
      onKeyDown={(e) => {
        if (!fx || status !== 'ready') return
        // Arrows step a frame back or on (held: keeps stepping); Space fires the effect again.
        if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
          if ((e.target as HTMLElement).tagName === 'INPUT') return
          e.preventDefault()
          return seek((frameNow + (e.key === 'ArrowLeft' ? -1 : 1)) * FRAME)
        }
        if (e.key !== ' ') return
        e.preventDefault()
        if (!e.repeat) fire()
      }}
    >
      <div className="viewer-canvas" ref={host} />
      {overlay && (
        <div className="viewer-overlay" role="status">
          {overlay}
        </div>
      )}
      {status === 'ready' && (
        <>
          <ViewGizmo viewer={viewer.current} />
          <div className="viewer-tools viewer-tools-top" role="toolbar" aria-label={t('viewer.tools')}>
            <button type="button" className="viewer-btn" onClick={() => viewer.current?.setView('reset')}>
              {t('viewer.view.reset')}
            </button>
            <button
              type="button"
              className="viewer-btn"
              aria-pressed={autoRotate}
              onClick={() => {
                viewer.current?.setAutoRotate(!autoRotate)
                setAutoRotate(!autoRotate)
              }}
            >
              {t('viewer.autoRotate')}
            </button>
            <button
              type="button"
              className="viewer-btn"
              aria-pressed={wireframe}
              onClick={() => {
                viewer.current?.setWireframe(!wireframe)
                setWireframe(!wireframe)
              }}
            >
              {t('viewer.wireframe')}
            </button>
            {fx && (
              <button
                type="button"
                className="viewer-btn"
                aria-pressed={backdrop === 'dark'}
                title={t('viewer.backdropHint')}
                onClick={() => {
                  const next = backdrop === 'dark' ? 'stage' : 'dark'
                  writeBackdrop(next)
                  setBackdrop(next)
                }}
              >
                {t('viewer.backdropDark')}
              </button>
            )}
          </div>
          {fx && (
            // Timeline above the play controls, stacked from the bottom: a narrow viewer wraps the controls without covering it.
            <div className="viewer-tools viewer-dock">
              <div className="viewer-row viewer-timeline" role="group" aria-label={t('particles.timeline')}>
                <button type="button" className="viewer-btn" title={t('particles.prevFrame')} aria-label={t('particles.prevFrame')} onClick={() => seek((frameNow - 1) * FRAME)}>
                  ‹
                </button>
                <input
                  type="range"
                  className="viewer-scrub"
                  min={0}
                  max={clock.length}
                  step={FRAME}
                  value={Math.min(clock.time, clock.length)}
                  aria-label={t('particles.timeline')}
                  onChange={(e) => seek(Number(e.target.value))}
                />
                <button type="button" className="viewer-btn" title={t('particles.nextFrame')} aria-label={t('particles.nextFrame')} onClick={() => seek((frameNow + 1) * FRAME)}>
                  ›
                </button>
                <span className="viewer-clock">
                  {clock.time.toFixed(2)}s · {t('particles.frame', { frame: frameNow })}
                </span>
              </div>
              <div className="viewer-row" role="toolbar" aria-label={t('particles.title')}>
                <button
                  type="button"
                  className="viewer-btn"
                  aria-pressed={!fx.paused}
                  onClick={() => {
                    // Play starts the effect over at once; pause holds it where it is.
                    if (fx.paused) built.current?.effect?.restart()
                    setEffect({ paused: !fx.paused })
                  }}
                >
                  {fx.paused ? t('particles.play') : t('particles.pause')}
                </button>
                <button type="button" className="viewer-btn" title={t('particles.fireHint')} onClick={fire}>
                  {t('particles.fire')}
                </button>
                <button type="button" className="viewer-btn" onClick={() => built.current?.effect?.restart()}>
                  {t('particles.restart')}
                </button>
                <span className="viewer-sep" aria-hidden="true" />
                <button type="button" className="viewer-btn" aria-pressed={fx.loop} title={t('particles.loopHint')} onClick={() => setEffect({ loop: !fx.loop })}>
                  {t('particles.loop')}
                </button>
                {/* Speed apart, on the right: one choice among three, drawn as a single control. */}
                <div className="viewer-seg" role="group" aria-label={t('particles.speed')}>
                  <span className="viewer-seg-label">{t('particles.speed')}</span>
                  {[0.25, 0.5, 1].map((s) => (
                    <button key={s} type="button" className="viewer-btn" aria-pressed={fx.speed === s} onClick={() => setEffect({ speed: s })}>
                      {s}×
                    </button>
                  ))}
                </div>
              </div>
            </div>
          )}
          <span className="viewer-hint">{missing > 0 ? t('prefabView.missing', { count: missing }) : t('viewer.hint')}</span>
        </>
      )}
    </div>
  )
}
