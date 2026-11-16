/** Stage timings for background thumbnails, to learn where a card's time goes. Off unless switched on. */
export function thumbTimingEnabled(): boolean {
  try {
    return localStorage.getItem('assethub.thumbTiming') === '1' || !!import.meta.env.DEV
  } catch {
    return false
  }
}

export interface ThumbTimer {
  /** Records the time since the previous mark (or the start) under this stage name. */
  mark: (stage: string) => void
  /** The last stage marked, or `start` before any: where a failure happened. */
  reached: () => string
  /** One log line: kind, path, total, then each stage in ms. */
  line: (kind: string, path: string, total?: number) => string
}

/** The clock is injected so the timer can be tested without waiting. */
export function createThumbTimer(now: () => number = () => performance.now(), startedAt = now()): ThumbTimer {
  const stages: [string, number][] = []
  let last = startedAt
  return {
    mark(stage) {
      const t = now()
      stages.push([stage, Math.round(t - last)])
      last = t
    },
    reached: () => stages[stages.length - 1]?.[0] ?? 'start',
    line(kind, path, total = Math.round(now() - startedAt)) {
      return `[thumb] ${kind} ${path} total=${total}${stages.map(([s, ms]) => ` ${s}=${ms}`).join('')}`
    }
  }
}
