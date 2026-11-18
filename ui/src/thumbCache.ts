/**
 * Thumbnails held in memory for the cards the user can see, read from the disk cache on demand.
 *
 * The disk cache can hold tens of thousands of data URLs (hundreds of MB as strings), so only what is
 * on screen, a little beyond it and the most recently seen ones stay in memory. A thumbnail that was
 * dropped from memory is still in the disk cache: it is read again when its card shows, and it is never a
 * reason to draw the thumbnail again. Only a lookup that came back empty (a "miss") allows drawing.
 */

/** Most thumbnails kept: about twenty screens of scroll-back, around 25 MB of data URLs at the average 12 KB. */
export const THUMB_CACHE_CAP = 2000
/** Keys per disk-cache request, so one request stays small however many cards appear at once. */
const BATCH_MAX = 256
/** Lookups wait this long to share a request (a short timer, not an animation frame, so it also runs in a hidden window). */
const BATCH_DELAY_MS = 16
/** A lookup that fails is tried again after this long, doubling each time; after `MAX_FAILURES` failures the card counts as having no thumbnail. */
const RETRY_BASE_MS = 500
const MAX_FAILURES = 3

export interface ThumbCacheOptions {
  /** Reads the saved thumbnails of `keys`: the data URL of each key that has one. */
  load(keys: string[]): Promise<Record<string, string>>
  /** Called after lookups answered or thumbnails were added, never for a plain eviction. */
  onChange(): void
  cap?: number
  delayMs?: number
}

declare global {
  /** Set to true by the test build's Vite config only; not defined in the app itself. */
  const __E2E__: boolean | undefined
  interface Window {
    /** Present only in the test build: how many thumbnails the cache holds. */
    __e2eThumbCount?: () => number
  }
}

export class ThumbCache {
  /** Insertion order is recency: the first entry is the least recently seen. */
  private readonly memory = new Map<string, string>()
  /** Lookups that found nothing in the disk cache: these cards may draw their thumbnail. */
  private readonly misses = new Set<string>()
  /** Cards on screen: never evicted. */
  private readonly visible = new Set<string>()
  /** The selected asset, the export list and an open collection: never evicted. */
  private kept = new Set<string>()
  private readonly pending = new Set<string>()
  private readonly inFlight = new Set<string>()
  /** Lookups whose request failed: how often, and when the next try may be made. */
  private readonly failures = new Map<string, { count: number; retryAt: number }>()
  private timer: ReturnType<typeof setTimeout> | undefined
  /** One timer for the earliest retry of a failed lookup. */
  private retryTimer: ReturnType<typeof setTimeout> | undefined
  private readonly cap: number
  private readonly delayMs: number

  constructor(private readonly options: ThumbCacheOptions) {
    this.cap = options.cap ?? THUMB_CACHE_CAP
    this.delayMs = options.delayMs ?? BATCH_DELAY_MS
  }

  get size(): number {
    return this.memory.size
  }

  get(sha: string): string | undefined {
    return this.memory.get(sha)
  }

  has(sha: string): boolean {
    return this.memory.has(sha)
  }

  /** The disk cache has no thumbnail for it. */
  isMiss(sha: string): boolean {
    return this.misses.has(sha) && !this.memory.has(sha)
  }

  /** The lookup finished: the thumbnail is in memory, or the disk cache has none. */
  answered(sha: string): boolean {
    return this.memory.has(sha) || this.misses.has(sha)
  }

  /** A card came on screen: it stays in memory until it leaves, and its lookup is queued when nothing is known yet. */
  request(sha: string): void {
    this.visible.add(sha)
    this.lookup(sha)
  }

  /** A card left the screen: it becomes the most recently seen of the evictable ones. */
  release(sha: string): void {
    this.visible.delete(sha)
    // Nobody waits for a failed lookup of a card that left: it is asked for again when the card shows again.
    if (!this.kept.has(sha)) this.failures.delete(sha)
    this.touch(sha)
    this.evict()
  }

  /** Looks up thumbnails ahead of the cards that need them, without keeping them. */
  prefetch(shas: Iterable<string>): void {
    for (const sha of shas) this.lookup(sha)
  }

  /** Replaces the set that is never evicted (and looks up what is missing from it). */
  keep(shas: Iterable<string>): void {
    this.kept = new Set(shas)
    for (const sha of this.failures.keys()) if (!this.visible.has(sha) && !this.kept.has(sha)) this.failures.delete(sha)
    for (const sha of this.kept) this.lookup(sha)
  }

  /** Thumbnails drawn this session. */
  putAll(found: Readonly<Record<string, string>>): void {
    const keys = Object.keys(found)
    if (!keys.length) return
    for (const key of keys) {
      this.misses.delete(key)
      this.insert(key, found[key])
    }
    this.evict()
    this.options.onChange()
  }

  /**
   * The saved thumbnail of each of `shas` that has one, from memory or else read from the disk cache in bounded requests.
   * Nothing read here is kept: it is for work on many assets at once (an export), which must not push the cards out of memory.
   */
  async saved(shas: readonly string[]): Promise<Record<string, string>> {
    const found: Record<string, string> = {}
    const missing: string[] = []
    for (const sha of new Set(shas)) {
      const url = this.memory.get(sha)
      if (url !== undefined) found[sha] = url
      else missing.push(sha)
    }
    for (let at = 0; at < missing.length; at += BATCH_MAX) {
      // A chunk that cannot be read leaves its assets without a preview; the export goes on.
      Object.assign(found, await this.options.load(missing.slice(at, at + BATCH_MAX)).catch(() => ({})))
    }
    return found
  }

  /** The thumbnails in memory as a plain object, for components that read `thumbs[sha]`. */
  snapshot(): Record<string, string> {
    return Object.fromEntries(this.memory)
  }

  private lookup(sha: string): void {
    if (this.memory.has(sha)) return this.touch(sha)
    if (this.misses.has(sha) || this.inFlight.has(sha) || this.pending.has(sha)) return
    if ((this.failures.get(sha)?.retryAt ?? 0) > Date.now()) {
      // The pause is not over: the timer asks again when it is, also for a card that came back on screen meanwhile.
      if (!this.retryTimer) this.armRetry()
      return
    }
    this.pending.add(sha)
    this.timer ??= setTimeout(() => this.flush(), this.delayMs)
  }

  private flush(): void {
    this.timer = undefined
    const keys = [...this.pending]
    this.pending.clear()
    for (let at = 0; at < keys.length; at += BATCH_MAX) {
      const chunk = keys.slice(at, at + BATCH_MAX)
      for (const key of chunk) this.inFlight.add(key)
      void this.options.load(chunk).then(
        (found) => this.answer(chunk, found),
        () => this.fail(chunk)
      )
    }
  }

  /**
   * The keys stay unanswered and a timer asks for them again after a pause (doubling each time), as long as their card
   * is on screen or they are kept. After `MAX_FAILURES` failures they count as having no thumbnail.
   */
  private fail(chunk: string[]): void {
    const now = Date.now()
    for (const key of chunk) {
      this.inFlight.delete(key)
      const count = (this.failures.get(key)?.count ?? 0) + 1
      if (count >= MAX_FAILURES) {
        this.failures.delete(key)
        this.misses.add(key)
      } else this.failures.set(key, { count, retryAt: now + RETRY_BASE_MS * 2 ** count })
    }
    this.armRetry()
    this.options.onChange()
  }

  /** Sets the retry timer for the earliest deadline among the failed lookups that nothing is running for. */
  private armRetry(): void {
    clearTimeout(this.retryTimer)
    this.retryTimer = undefined
    let next = Infinity
    for (const [key, failure] of this.failures) {
      if (!this.inFlight.has(key) && !this.pending.has(key)) next = Math.min(next, failure.retryAt)
    }
    if (next < Infinity) this.retryTimer = setTimeout(() => this.retry(), Math.max(0, next - Date.now()))
  }

  private retry(): void {
    this.retryTimer = undefined
    const now = Date.now()
    for (const [key, failure] of [...this.failures]) {
      if (!this.visible.has(key) && !this.kept.has(key)) this.failures.delete(key)
      else if (failure.retryAt <= now) this.lookup(key)
    }
    this.armRetry()
  }

  /** Stops the timers (the cache is no longer used); lookups still waiting for their timer are asked for again when their card shows. */
  dispose(): void {
    clearTimeout(this.timer)
    clearTimeout(this.retryTimer)
    this.timer = this.retryTimer = undefined
    this.pending.clear()
  }

  private answer(chunk: string[], found: Readonly<Record<string, string>>): void {
    for (const key of chunk) {
      this.inFlight.delete(key)
      this.failures.delete(key)
      // A thumbnail drawn while the lookup ran is newer than the answer.
      if (this.memory.has(key)) continue
      if (found[key]) this.insert(key, found[key])
      else this.misses.add(key)
    }
    this.evict()
    this.options.onChange()
  }

  private insert(sha: string, url: string): void {
    this.memory.delete(sha)
    this.memory.set(sha, url)
  }

  private touch(sha: string): void {
    const url = this.memory.get(sha)
    if (url !== undefined) this.insert(sha, url)
  }

  private evict(): void {
    if (this.memory.size <= this.cap) return
    for (const sha of this.memory.keys()) {
      if (this.memory.size <= this.cap) return
      if (!this.visible.has(sha) && !this.kept.has(sha)) this.memory.delete(sha)
    }
  }
}
