import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import * as THREE from 'three'
import { useTranslation } from 'react-i18next'
import type { DependencyReport, ModelStats } from '@shared/types'
import { ViewGizmo } from './ViewGizmo'
import { LunaNotes, LunaView, type LunaHandle, type LunaNote, type LunaScene } from './LunaView'
import { FbxViewer } from '../viewer/FbxViewer'
import { buildPrefab, dressPrefab, type PrefabBuild } from '../viewer/prefabBuild'
import { buildEffect, closeLunaTextures, type BuiltEffect } from '../viewer/effectBuild'
import { FRAME } from '../viewer/particlePlayer'
import { buildLunaEffect, nodesOf, type TransformLike } from '../luna/bridge'
import { frameEffect } from '../luna/framing'
import type { LunaCamera, Rgba } from '../luna/protocol'
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

/** Which player shows an effect: the viewer's own (Unity), the Luna runtime, or both side by side. */
export type FxMode = 'unity' | 'luna' | 'split'
const MODE_KEY = 'assethub.fxMode'
const MODES: readonly FxMode[] = ['unity', 'luna', 'split']

/** Remembered per machine like the backdrop; storage can be unavailable (then it is Unity). */
function readMode(): FxMode {
  try {
    const value = localStorage.getItem(MODE_KEY)
    return MODES.find((m) => m === value) ?? 'unity'
  } catch {
    return 'unity'
  }
}

function writeMode(value: FxMode): void {
  try {
    localStorage.setItem(MODE_KEY, value)
  } catch {
    // Not remembered: still applies until the app closes.
  }
}

/** The colour the viewer shows behind an effect, from the page's own variables so that Luna draws the same one. */
function backdropColor(backdrop: FxBackdrop): Rgba {
  const hex = getComputedStyle(document.documentElement).getPropertyValue(backdrop === 'dark' ? '--ground' : '--stage').trim()
  const n = /^#([0-9a-f]{6})$/i.exec(hex)?.[1]
  const value = n ? parseInt(n, 16) : backdrop === 'dark' ? 0x1f1f20 : 0x767676
  return [((value >> 16) & 255) / 255, ((value >> 8) & 255) / 255, (value & 255) / 255, 1]
}

const IDENTITY: Pick<TransformLike, 'position' | 'quaternion' | 'scale' | 'visible'> = { position: { x: 0, y: 0, z: 0 }, quaternion: { x: 0, y: 0, z: 0, w: 1 }, scale: { x: 1, y: 1, z: 1 }, visible: true }

/** What the Luna pane needs of a built prefab: the same emitters, materials, textures and hierarchy the Unity-style player has. */
function lunaScene(effect: BuiltEffect, build: PrefabBuild, camera: LunaCamera | undefined): LunaScene {
  const linear = !effect.luna.gamma
  return {
    linear,
    camera,
    build: async () => {
      const materials = await effect.luna.materials()
      try {
        // A system on the prefab's own root has no object below the group that stands for the world: it gets one of its own.
        const rootNode: TransformLike = { name: 'Root', parent: build.group, ...IDENTITY }
        const objects = build.anchors.map((a) => (a.object === build.group ? rootNode : a.object))
        const { nodes, indexOf } = nodesOf(objects, build.group)
        const spec = buildLunaEffect({
          linear,
          nodes,
          emitters: build.anchors.map(({ emitter }, i) => ({ emitter, node: indexOf.get(objects[i])!, material: materials[i] }))
        })
        return { spec, release: () => closeLunaTextures(materials) }
      } catch (err) {
        closeLunaTextures(materials)
        throw err
      }
    }
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
  const built = useRef<{ key: string; build: PrefabBuild; effect?: BuiltEffect; stop?: () => void }>()
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
  const [mode, setMode] = useState(readMode)
  /** The effect as the Luna pane takes it, set with each build. */
  const [scene, setScene] = useState<LunaScene>()
  const [notes, setNotes] = useState<LunaNote[]>([])
  const [notesOpen, setNotesOpen] = useState(false)
  const luna = useRef<LunaHandle>(null)
  /** The runtime starts when Luna is first asked for and stays until the viewer closes. */
  const [lunaUsed, setLunaUsed] = useState(mode !== 'unity')
  /** The Unity-style player's time at the last frame, to see it start over by itself (a loop). */
  const lastTime = useRef(0)
  // Only effects have the choice; a model always sits on the stage.
  const shownBackdrop: FxBackdrop = fx ? backdrop : 'stage'
  const callbacks = useRef({ onStats, onThumb, onLoading, onBackdrop })
  callbacks.current = { onStats, onThumb, onLoading, onBackdrop }
  const fxNow = useRef(fx)
  fxNow.current = fx
  // Only prefabs with particles can show Luna.
  const shownMode: FxMode = fx ? mode : 'unity'
  const lunaShown = shownMode !== 'unity'
  /** The updater below lives as long as a build, so it reads whether Luna is shown from here. */
  const lunaShownRef = useRef(lunaShown)
  lunaShownRef.current = lunaShown
  /** The Luna pane's backdrop colour, read once the page shows that backdrop (not on every render). */
  const [lunaBackground, setLunaBackground] = useState(() => backdropColor(shownBackdrop))
  useLayoutEffect(() => setLunaBackground(backdropColor(shownBackdrop)), [shownBackdrop])
  /** A seek waiting for the next frame, so a drag over the timeline sends one per frame, not one per event. */
  const pendingSeek = useRef<{ frame: number; time: number }>()
  useEffect(() => () => {
    if (pendingSeek.current) cancelAnimationFrame(pendingSeek.current.frame)
  }, [])
  // Once the runtime has started it stays mounted, with or without an effect, so it survives a change of prefab.
  const lunaMounted = useRef(false)
  if (lunaUsed && scene) lunaMounted.current = true

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
    // Everything that belongs to the previous prefab goes: the clock, the controls, the notices and its effect.
    setStatus('waiting')
    setError('')
    setClock({ time: 0, length: 1 })
    lastTime.current = 0
    setMissing(0)
    setFx(undefined)
    setScene(undefined)
    setNotes([])
    setNotesOpen(false)
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

  // Luna alone shows the effect: the Unity-style view stops drawing (its clock keeps running) until it is shown again.
  useEffect(() => {
    viewer.current?.setRendering(shownMode !== 'luna')
  }, [shownMode, live.repo, live.branch, live.root, path])

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
              // The Unity-style player starts a finished effect over by itself: Luna starts over with it.
              // Only while Luna is shown: a pane that is not shown holds no effect.
              if (lunaShownRef.current && !effect.paused && effect.time < lastTime.current - 1e-6) luna.current?.send({ type: 'restart' })
              lastTime.current = effect.time
              // The timeline follows the play, one render per frame of it at most.
              const at = Math.floor(effect.time / FRAME)
              if (at !== shown) {
                shown = at
                setClock({ time: effect.time, length: effect.timeline() })
              }
            })
        }
        // The camera takes in where particles fly, not only the meshes.
        const extent = effect?.measuredExtent()
        // The same box the viewer frames (it moves the group when it takes it), so Luna can frame the same view.
        const box = new THREE.Box3().setFromObject(build.group)
        if (extent) box.union(extent)
        owner.showGroup(build.group, extent)
        await dressPrefab(owner, build)
        previous?.build.dispose()
        // The viewer may have moved to another prefab meanwhile, and its cleanup already disposed this build.
        if (cancelled || owner !== viewer.current || owner.isDisposed()) return
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
          lastTime.current = 0
          setScene(lunaScene(effect, build, frameEffect(box)))
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
    // Pausing holds both players where they are. Resuming goes on from there (a fired shot needs the running effect).
    if (patch.paused !== undefined) sendLuna({ type: patch.paused ? 'pause' : 'play' })
    setFx(next)
  }
  const sendLuna: LunaHandle['send'] = (command) => {
    if (!lunaShown) return
    if (command.type === 'seek') {
      // The last time of the frame wins.
      const pending = pendingSeek.current
      if (pending) pending.time = command.time
      else {
        const next = { frame: 0, time: command.time }
        next.frame = requestAnimationFrame(() => {
          pendingSeek.current = undefined
          luna.current?.send({ type: 'seek', time: next.time })
        })
        pendingSeek.current = next
      }
      return
    }
    // A command after a seek must find the effect where the seek put it.
    flushSeek()
    luna.current?.send(command)
  }
  const flushSeek = () => {
    const pending = pendingSeek.current
    if (!pending) return
    cancelAnimationFrame(pending.frame)
    pendingSeek.current = undefined
    luna.current?.send({ type: 'seek', time: pending.time })
  }
  /** Plays the effect from its start in both players. */
  const restart = () => {
    built.current?.effect?.restart()
    lastTime.current = 0
    sendLuna({ type: 'restart' })
  }
  /**
   * The Luna pane has its effect (it loads when the pane is shown): puts it on the Unity-style player's clock, which
   * keeps its own time. The player restarts Luna at each loop, so the host does not loop by itself.
   */
  const syncLuna = () => {
    const effect = built.current?.effect
    const state = fxNow.current
    if (!lunaShownRef.current || !effect || !state) return
    luna.current?.send({ type: 'loop', value: false })
    luna.current?.send({ type: 'seek', time: effect.time })
    if (!state.paused) luna.current?.send({ type: 'play' })
  }
  const chooseMode = (next: FxMode) => {
    writeMode(next)
    setMode(next)
    if (next !== 'unity') setLunaUsed(true)
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
    lastTime.current = effect.time
    // Luna restarts and steps to the same time with `Simulate`, which leaves it paused there.
    sendLuna({ type: 'seek', time: effect.time })
    setClock({ time: effect.time, length })
  }
  const frameNow = Math.round(clock.time / FRAME)

  /** Another play of the effect on top of the running ones, like spawning the prefab again (spam it like in Unity). */
  const fire = () => {
    const effect = built.current?.effect
    if (!effect || !fx) return
    if (fx.paused) setEffect({ paused: false })
    effect.fire()
    sendLuna({ type: 'fire' })
  }

  return (
    <div
      className="viewer"
      data-backdrop={shownBackdrop}
      data-mode={shownMode}
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
      {lunaMounted.current && (
        <LunaView
          ref={luna}
          scene={scene}
          background={lunaBackground}
          speed={fx?.speed ?? 1}
          hidden={!lunaShown || !scene}
          className="viewer-luna"
          onLoaded={syncLuna}
          onNotes={setNotes}
        />
      )}
      {overlay && (
        <div className="viewer-overlay" role="status">
          {overlay}
        </div>
      )}
      {status === 'ready' && (
        <>
          {shownMode !== 'luna' && <ViewGizmo viewer={viewer.current} />}
          <div className="viewer-tools viewer-tools-top" role="toolbar" aria-label={t('viewer.tools')}>
            {shownMode !== 'luna' && (
              <>
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
              </>
            )}
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
            {fx && (
              <div className="viewer-seg" role="group" aria-label={t('luna.mode')} title={t('luna.modeHint')}>
                <span className="viewer-seg-label">{t('luna.mode')}</span>
                {MODES.map((m) => (
                  <button key={m} type="button" className="viewer-btn" aria-pressed={mode === m} onClick={() => chooseMode(m)}>
                    {t(`luna.modes.${m}`)}
                  </button>
                ))}
              </div>
            )}
            {lunaShown && <LunaNotes notes={notes} open={notesOpen} onToggle={() => setNotesOpen(!notesOpen)} />}
          </div>
          {shownMode === 'split' && (
            <>
              <span className="viewer-tag viewer-tag-unity">{t('luna.modes.unity')}</span>
              <span className="viewer-tag viewer-tag-luna">{t('luna.modes.luna')}</span>
            </>
          )}
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
                    if (fx.paused) restart()
                    setEffect({ paused: !fx.paused })
                  }}
                >
                  {fx.paused ? t('particles.play') : t('particles.pause')}
                </button>
                <button type="button" className="viewer-btn" title={t('particles.fireHint')} onClick={fire}>
                  {t('particles.fire')}
                </button>
                <button type="button" className="viewer-btn" onClick={restart}>
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
          {shownMode !== 'split' && (missing > 0 || shownMode === 'unity') && <span className="viewer-hint">{missing > 0 ? t('prefabView.missing', { count: missing }) : t('viewer.hint')}</span>}
        </>
      )}
    </div>
  )
}
