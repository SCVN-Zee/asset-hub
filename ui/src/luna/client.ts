/** App-window side of the Luna preview: owns the host iframe, sends commands and exposes the events. */
import { LUNA_HOST_URL } from './runtime-info'
import { envelope, isEnvelope, type LunaCommand, type LunaEvent } from './protocol'

/** The runtime loads about 8 MB of scripts and compiles shaders, so this is generous. */
const READY_TIMEOUT_MS = 20000

export type LunaListener = (event: LunaEvent) => void

export class LunaClient {
  /** Resolves with the runtime version when the host is ready. Rejects when the host cannot run or reports an error first. */
  readonly ready: Promise<string>
  private readonly frame: HTMLIFrameElement
  private readonly listeners = new Set<LunaListener>()
  private isReady = false
  private disposed = false

  /**
   * Adds the iframe to `parent` and starts the runtime in it. The caller sizes the parent. `linear` starts the runtime in
   * linear colour space, as the project draws, and only effects built for the same space load.
   */
  constructor(parent: HTMLElement, linear = false) {
    this.frame = document.createElement('iframe')
    this.frame.title = 'Luna preview'
    this.frame.style.cssText = 'border:0;width:100%;height:100%;display:block'

    this.ready = new Promise<string>((resolve, reject) => {
      const timer = window.setTimeout(() => reject(new Error('The Luna runtime did not start in time')), READY_TIMEOUT_MS)
      const off = this.on((event) => {
        if (event.type === 'ready') {
          this.isReady = true
          resolve(event.version)
        } else if (event.type === 'unsupported' && !this.isReady) {
          reject(new Error(event.detail))
        } else if (event.type === 'error' && !this.isReady) {
          reject(new Error(event.message))
        } else {
          return
        }
        window.clearTimeout(timer)
        off()
      })
    })
    // A rejection nobody awaits yet must not surface as an unhandled rejection.
    this.ready.catch(() => {})

    window.addEventListener('message', this.onMessage)
    this.frame.src = linear ? `${LUNA_HOST_URL}?space=linear` : LUNA_HOST_URL
    parent.appendChild(this.frame)
  }

  /** True from the `ready` event until `dispose`: the host takes commands. */
  get running(): boolean {
    return this.isReady && !this.disposed
  }

  /** Subscribes to host events. Returns the unsubscribe function. */
  on(listener: LunaListener): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  /** Sends a command. Throws before the host is ready, so await `ready` first. */
  send(command: LunaCommand): void {
    const target = this.frame.contentWindow
    if (this.disposed || !this.isReady || !target) throw new Error('The Luna host is not ready')
    target.postMessage(envelope(command), window.location.origin)
  }

  /** Removes the iframe, which ends the runtime and frees its WebGL context. */
  dispose(): void {
    if (this.disposed) return
    this.disposed = true
    window.removeEventListener('message', this.onMessage)
    this.listeners.clear()
    this.frame.remove()
  }

  private readonly onMessage = (event: MessageEvent): void => {
    if (event.source !== this.frame.contentWindow || event.origin !== window.location.origin || !isEnvelope(event.data)) return
    const message = event.data.message as LunaEvent
    for (const listener of [...this.listeners]) listener(message)
  }
}
