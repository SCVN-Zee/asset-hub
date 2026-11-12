import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { LunaClient } from '../luna/client'
import type { LunaCamera, LunaCommand, LunaEffectSpec, Rgba } from '../luna/protocol'

/** What the Luna pane shows: one effect, ready to hand to the runtime. A new object loads again. */
export interface LunaScene {
  /** The project draws in linear colour space. The runtime starts in that space, and a change of it starts a new runtime. */
  linear: boolean
  /** Where to put the camera. Undefined keeps the runtime's own view. */
  camera?: LunaCamera
  /** Makes the description. `release` frees the textures it holds, once the host has them. */
  build(): Promise<{ spec: LunaEffectSpec; release(): void }>
}

/** Something Luna does not draw as Unity does. */
export interface LunaNote {
  /** The particle system it concerns. Absent for the whole effect. */
  emitter?: string
  /** The module the host names, or `shader` for a stand-in shader, or `material`. */
  what: string
  detail: string
}

export interface LunaHandle {
  /** Sends a command. Commands before the runtime is ready are dropped, so sync again from `onLoaded`. */
  send(command: LunaCommand): void
}

interface Props {
  scene?: LunaScene
  /** Colour behind the effect, as drawn on screen. */
  background: Rgba
  /** Playback speed, 1 is normal. */
  speed: number
  /** Not shown. The runtime stays up, but it holds no effect: the effect is removed from it, and loads again when shown. */
  hidden: boolean
  /** Layout class of the pane, which the viewer places. */
  className: string
  /** The effect is in the runtime and framed: the owner syncs the clock now. */
  onLoaded?(): void
  /** The list of differences found while loading, whenever it grows. */
  onNotes?(notes: LunaNote[]): void
}

type Phase = { name: 'starting' } | { name: 'idle' } | { name: 'loading' } | { name: 'loaded' } | { name: 'error'; message: string }

/**
 * The Luna runtime in an isolated iframe. The runtime starts on first use and stays across effects (a new `scene` loads
 * into it), and it ends when this component unmounts.
 */
export const LunaView = forwardRef<LunaHandle, Props>(function LunaView({ scene, background, speed, hidden, className, onLoaded, onNotes }, ref) {
  const { t } = useTranslation()
  const frame = useRef<HTMLDivElement>(null)
  const client = useRef<LunaClient>()
  const [phase, setPhase] = useState<Phase>({ name: 'starting' })
  /** The client that is up and takes commands. Another client (a new colour space, a retry) is not up until it says so. */
  const [booted, setBooted] = useState<LunaClient>()
  const [attempt, setAttempt] = useState(0)
  const latest = useRef({ background, speed, onLoaded, onNotes })
  latest.current = { background, speed, onLoaded, onNotes }
  // Between two effects there is no scene: the runtime stays on the colour space it started in.
  const space = useRef<boolean>()
  if (scene) space.current = scene.linear
  const linear = space.current
  /** The runtime holds the effect only while the pane shows it. */
  const active = !hidden && !!scene
  /** Counts loads, so the answers of one that was replaced are ignored. */
  const loadId = useRef(0)

  useImperativeHandle(ref, () => ({
    send: (command) => {
      if (client.current?.running) client.current.send(command)
    }
  }))

  // Start the runtime on first use. The colour space is fixed when it starts, so another one starts a new runtime.
  useEffect(() => {
    const parent = frame.current
    if (linear === undefined || !parent) return
    const c = new LunaClient(parent, linear)
    client.current = c
    let live = true
    setBooted(undefined)
    setPhase({ name: 'starting' })
    c.ready.then(
      () => live && setBooted(c),
      (err) => live && setPhase({ name: 'error', message: err instanceof Error ? err.message : String(err) })
    )
    return () => {
      live = false
      c.dispose()
      if (client.current === c) client.current = undefined
    }
  }, [linear, attempt])

  // Load the effect when the pane shows a scene and the runtime is up. When either stops, the effect leaves the runtime,
  // so an effect nobody sees does not run in the background.
  useEffect(() => {
    const c = client.current
    if (!scene || !active || !c || booted !== c) return
    const id = ++loadId.current
    let live = true
    let release: (() => void) | undefined
    const found: LunaNote[] = []
    const seen = new Set<string>()
    setPhase({ name: 'loading' })
    latest.current.onNotes?.([])
    const off = c.on((event) => {
      if (!live) return
      // Answers of an earlier load carry its id. Runtime errors carry none and always count.
      if ('id' in event && event.id !== undefined && event.id !== id) return
      if (event.type === 'unsupported') {
        const key = `${event.emitter ?? ''}\n${event.what}\n${event.detail}`
        if (seen.has(key)) return
        seen.add(key)
        found.push({ emitter: event.emitter, what: event.what, detail: event.detail })
        latest.current.onNotes?.([...found])
      } else if (event.type === 'error') {
        setPhase({ name: 'error', message: event.message })
      } else if (event.type === 'loaded') {
        if (scene.camera) c.send({ type: 'camera', camera: scene.camera })
        c.send({ type: 'background', color: latest.current.background })
        c.send({ type: 'speed', value: latest.current.speed })
        setPhase({ name: 'loaded' })
        latest.current.onLoaded?.()
      }
    })
    scene.build().then(
      (built) => {
        release = built.release
        try {
          if (live) c.send({ type: 'load', id, effect: built.spec })
        } catch (err) {
          if (live) setPhase({ name: 'error', message: err instanceof Error ? err.message : String(err) })
        } finally {
          // The host has its own copy of the textures once the message is sent.
          built.release()
          release = undefined
        }
      },
      (err) => live && setPhase({ name: 'error', message: err instanceof Error ? err.message : String(err) })
    )
    return () => {
      live = false
      off()
      release?.()
      setPhase({ name: 'idle' })
      try {
        if (c.running) c.send({ type: 'dispose' })
      } catch {
        // The runtime is already gone.
      }
    }
  }, [scene, active, booted])

  const loaded = phase.name === 'loaded'
  useEffect(() => {
    if (loaded) client.current?.send({ type: 'background', color: background })
  }, [background, loaded])
  useEffect(() => {
    if (loaded) client.current?.send({ type: 'speed', value: speed })
  }, [speed, loaded])

  const overlay = phase.name === 'starting' ? t('luna.starting') : phase.name === 'loading' ? t('luna.loading') : undefined
  return (
    <div className={`luna-pane ${className}`} hidden={hidden}>
      <div className="luna-frame" ref={frame} />
      {overlay && (
        <div className="viewer-overlay" role="status">
          {overlay}
        </div>
      )}
      {phase.name === 'error' && (
        <div className="viewer-overlay luna-error" role="alert">
          <span>{t('luna.error', { message: phase.message })}</span>
          <button type="button" className="viewer-btn" onClick={() => setAttempt((a) => a + 1)}>
            {t('luna.retry')}
          </button>
        </div>
      )}
    </div>
  )
})

/** The things Luna draws differently, as a button for the toolbar and a list that opens under it. */
export function LunaNotes({ notes, open, onToggle }: { notes: readonly LunaNote[]; open: boolean; onToggle(): void }) {
  const { t } = useTranslation()
  if (notes.length === 0) return null
  // The whole effect first, then each particle system in the order it first appears.
  const groups = new Map<string, LunaNote[]>()
  for (const note of notes) groups.set(note.emitter ?? '', [...(groups.get(note.emitter ?? '') ?? []), note])
  const ordered = [...groups.entries()].sort(([a], [b]) => (a === '' ? -1 : b === '' ? 1 : 0))
  return (
    <>
      <button type="button" className="viewer-btn viewer-btn-warn" aria-expanded={open} title={t('luna.differsHint')} onClick={onToggle}>
        {t('luna.differs', { count: notes.length })}
      </button>
      {open && (
        <div className="luna-notes" role="region" aria-label={t('luna.notesTitle')}>
          {ordered.map(([emitter, items]) => (
            <section key={emitter}>
              <h4>{emitter || t('luna.wholeEffect')}</h4>
              <ul>
                {items.map((n) => (
                  <li key={`${n.what}\n${n.detail}`}>{n.what === 'shader' ? t('luna.shaderFallback', { name: n.detail }) : n.what === 'material' ? n.detail : `${n.what}: ${n.detail}`}</li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
    </>
  )
}
