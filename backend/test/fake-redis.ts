import type { Ranked, SafeRedis } from '../src/redis'

/** In-memory stand-in for the Redis wrapper. TTLs follow `Date.now()`, so fake timers move them. */
export class FakeRedis implements SafeRedis {
  down = false
  /** Names of the commands received, in order. */
  calls: string[] = []
  private readonly values = new Map<string, { value: string; expiresAt: number }>()
  private readonly zsets = new Map<string, Map<string, number>>()
  /** Rankings written by `replaceRankings`. */
  replaced: Array<{ key: string; members: Ranked[]; builtAt: number }> = []

  /** Number of rankings in each `replaceRankings` call. */
  batches: number[] = []

  reset(): void {
    this.batches = []
    this.down = false
    this.calls = []
    this.replaced = []
    this.values.clear()
    this.zsets.clear()
  }

  async get(key: string): Promise<string | null | undefined> {
    this.calls.push('get')
    if (this.down) return undefined
    const hit = this.values.get(key)
    if (!hit || hit.expiresAt <= Date.now()) return null
    return hit.value
  }

  async set(key: string, value: string, ttlSec: number): Promise<boolean | undefined> {
    this.calls.push('set')
    if (this.down) return undefined
    this.values.set(key, { value, expiresAt: Date.now() + ttlSec * 1000 })
    return true
  }

  async zincrby(key: string, increment: number, member: string): Promise<number | undefined> {
    this.calls.push('zincrby')
    if (this.down) return undefined
    const set = this.zsets.get(key) ?? new Map<string, number>()
    const next = (set.get(member) ?? 0) + increment
    set.set(member, next)
    this.zsets.set(key, set)
    return next
  }

  async zrevrange(key: string, start: number, stop: number): Promise<Array<{ member: string; score: number }> | undefined> {
    this.calls.push('zrevrange')
    if (this.down) return undefined
    const set = this.zsets.get(key)
    if (!set) return []
    return [...set.entries()]
      .map(([member, score]) => ({ member, score }))
      .sort((a, b) => b.score - a.score)
      .slice(start, stop + 1)
  }

  async ranking(key: string, count: number): Promise<{ items: Ranked[]; builtAt: number | null } | undefined> {
    this.calls.push('ranking')
    if (this.down) return undefined
    const items = [...(this.zsets.get(key) ?? new Map<string, number>()).entries()]
      .map(([member, score]) => ({ member, score }))
      .sort((a, b) => b.score - a.score)
      .slice(0, count)
    const at = this.values.get(`${key}:at`)
    return { items, builtAt: at ? Number(at.value) : null }
  }

  async replaceRankings(rankings: Array<{ key: string; members: Ranked[] }>, builtAt: number, ttlSec: number): Promise<number> {
    this.calls.push('replaceRankings')
    if (this.down) return rankings.length * 5
    this.batches.push(rankings.length)
    for (const { key, members } of rankings) {
      this.replaced.push({ key, members, builtAt })
      this.zsets.set(key, new Map(members.map((m) => [m.member, m.score])))
      this.values.set(`${key}:at`, { value: String(builtAt), expiresAt: Date.now() + ttlSec * 1000 })
    }
    return 0
  }
}

export const fakeRedis = new FakeRedis()