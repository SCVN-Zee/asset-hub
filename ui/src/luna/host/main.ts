/** Entry of `luna-host.html`: boots the Luna runtime and answers the commands of the app window. */
import { LUNA_VERSION } from '../runtime-info'
import { envelope, isEnvelope, type LunaCommand, type LunaEvent } from '../protocol'
import { bootEngine, loadRuntime } from './boot'
import { EffectPreview } from './effect'

/** Seconds between `time` events. */
const TICK_MS = 100
/** Errors reported at most, so a failing frame loop does not flood the app. */
const MAX_ERRORS = 5

let errorCount = 0

function send(event: LunaEvent): void {
  window.parent.postMessage(envelope(event), window.location.origin)
}

function describe(error: unknown): string {
  // Luna wraps C# exceptions. The useful stack is on `errorStack`.
  const e = error as { errorStack?: { stack?: string }; stack?: string; message?: string } | null
  return String(e?.errorStack?.stack ?? e?.stack ?? e?.message ?? error)
}

function sendError(error: unknown, id?: number): void {
  if (errorCount++ < MAX_ERRORS) send({ type: 'error', message: describe(error), id })
}

function hasWebGL2(): boolean {
  return document.createElement('canvas').getContext('webgl2') !== null
}

async function main(): Promise<void> {
  if (!hasWebGL2()) {
    send({ type: 'unsupported', what: 'webgl2', detail: 'This web view has no WebGL 2, which the Luna runtime needs.' })
    return
  }
  await loadRuntime()
  // The page URL picks the colour space, because the engine fixes it at start.
  const linear = new URLSearchParams(window.location.search).get('space') === 'linear'
  const engine = await bootEngine(document.getElementById('application-canvas') as HTMLCanvasElement, linear)
  /** The load in progress: what the host reports while building it carries its id. */
  let loadId: number | undefined
  const preview = new EffectPreview(engine, (what, detail, emitter) => send({ type: 'unsupported', what, detail, emitter, id: loadId }), linear)

  const load = (command: Extract<LunaCommand, { type: 'load' }>): void => {
    loadId = command.id
    try {
      send({ type: 'loaded', textureMs: preview.load(command.effect), id: loadId })
    } catch (error) {
      sendError(error, loadId)
    } finally {
      // The engine has its own copy of every texture now, so the bitmaps this message brought can go.
      for (const m of command.effect.materials) for (const t of Object.values(m.textures)) if (t && 'bitmap' in t) t.bitmap.close()
    }
  }

  const handle = (command: LunaCommand): void => {
    switch (command.type) {
      case 'load': return load(command)
      case 'camera': return preview.setCamera(command.camera)
      case 'background': return preview.setBackground(command.color)
      case 'speed': return preview.setSpeed(command.value)
      case 'loop': return preview.setLoop(command.value)
      case 'play': return preview.play()
      case 'pause': return preview.pause()
      case 'restart': return preview.restart()
      case 'seek': return preview.seek(command.time)
      case 'fire': return preview.fire()
      case 'dispose': return preview.dispose()
    }
  }

  window.addEventListener('message', (event) => {
    // Only the embedding app window may command the host.
    if (event.source !== window.parent || event.origin !== window.location.origin || !isEnvelope(event.data)) return
    try {
      handle(event.data.message as LunaCommand)
    } catch (error) {
      sendError(error)
    }
  })
  window.setInterval(() => {
    try {
      if (preview.loaded) send({ type: 'time', ...preview.tick() })
    } catch (error) {
      sendError(error)
    }
  }, TICK_MS)
  send({ type: 'ready', version: LUNA_VERSION })
}

window.addEventListener('error', (event) => sendError(event.error ?? event.message))
window.addEventListener('unhandledrejection', (event) => sendError(event.reason))
main().catch(sendError)
