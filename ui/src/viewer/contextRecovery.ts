/** What the recovery needs from the page, so it can be tested without a web view. */
export interface RecoveryPage {
  /** True while the window is hidden or covered (animation frames stop and WebKit reclaims idle contexts). */
  hidden(): boolean
  /** Calls `fn` once, the next time the page becomes visible. */
  whenVisible(fn: () => void): void
  /** Calls `fn` after the event that reported the loss has finished. */
  later(fn: () => void): void
  now(): number
}

/** At most this many rebuilds in the window: a context that keeps getting lost is left alone. */
const MAX_REBUILDS = 3
const WINDOW_MS = 10_000

/**
 * Brings a viewer back after WebKit took its WebGL context away. It does that when many other
 * contexts are made while the window is hidden (thumbnails each draw with one), and a lost context
 * is not given back: the viewer would stay dark for good. `rebuild` makes a new renderer; it runs
 * once the page is visible again, because a hidden page would only lose the new one too.
 * Returns what to call on every `webglcontextlost` event.
 */
export function contextRecovery(rebuild: () => void, page: RecoveryPage): () => void {
  const recent: number[] = []
  let waiting = false
  const attempt = (): void => {
    if (page.hidden()) return page.whenVisible(attempt)
    waiting = false
    const now = page.now()
    while (recent.length && now - recent[0] > WINDOW_MS) recent.shift()
    if (recent.length >= MAX_REBUILDS) return
    recent.push(now)
    rebuild()
  }
  return () => {
    if (waiting) return
    waiting = true
    page.later(attempt)
  }
}
