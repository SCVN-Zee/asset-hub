import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ThumbCache } from './thumbCache'

/** A disk cache that holds `disk`, and records the size of every request. */
function setup(disk: Record<string, string>, cap = 3) {
  const requests: string[][] = []
  const changes = { count: 0 }
  const cache = new ThumbCache({
    load: (keys) => {
      requests.push(keys)
      return Promise.resolve(Object.fromEntries(keys.filter((k) => k in disk).map((k) => [k, disk[k]])))
    },
    onChange: () => changes.count++,
    cap,
    delayMs: 10
  })
  return { cache, requests, changes }
}

const settle = async () => {
  await vi.advanceTimersByTimeAsync(20)
}

describe('ThumbCache', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('joins lookups made close together into one request and asks once per key', async () => {
    const { cache, requests } = setup({ a: 'A' })
    cache.request('a')
    cache.request('b')
    cache.request('a')
    cache.prefetch(['b', 'c'])
    await settle()
    expect(requests).toEqual([['a', 'b', 'c']])
    cache.request('c')
    await settle()
    expect(requests).toHaveLength(1)
  })

  it('splits a large lookup into requests of bounded size', async () => {
    const { cache, requests } = setup({})
    cache.prefetch(Array.from({ length: 600 }, (_, i) => `k${i}`))
    await settle()
    expect(requests.map((r) => r.length)).toEqual([256, 256, 88])
  })

  it('tells an answered miss from a lookup still running, and a miss from a thumbnail', async () => {
    const { cache } = setup({ a: 'A' })
    cache.request('a')
    cache.request('b')
    expect(cache.answered('a')).toBe(false)
    expect(cache.isMiss('b')).toBe(false)
    await settle()
    expect(cache.answered('a')).toBe(true)
    expect(cache.get('a')).toBe('A')
    expect(cache.isMiss('a')).toBe(false)
    expect(cache.answered('b')).toBe(true)
    expect(cache.isMiss('b')).toBe(true)
  })

  it('evicts the least recently seen thumbnail and never one on screen or kept', async () => {
    const { cache } = setup({ a: 'A', b: 'B', c: 'C', d: 'D', e: 'E', f: 'F' })
    cache.request('a')
    cache.keep(['b'])
    cache.prefetch(['c', 'd'])
    await settle()
    // Over the cap by one: `a` is on screen and `b` is kept, so the oldest of the rest goes.
    expect([cache.has('a'), cache.has('b'), cache.has('c'), cache.has('d')]).toEqual([true, true, false, true])
    cache.prefetch(['e'])
    await settle()
    expect([cache.has('a'), cache.has('b'), cache.has('d'), cache.has('e')]).toEqual([true, true, false, true])
    cache.release('a')
    cache.prefetch(['f'])
    await settle()
    expect([cache.has('a'), cache.has('b'), cache.has('e'), cache.has('f')]).toEqual([true, true, false, true])
    expect(cache.size).toBe(3)
  })

  it('reads an evicted thumbnail again when its card shows, and does not treat it as a miss', async () => {
    const { cache, requests, changes } = setup({ a: 'A', b: 'B', c: 'C', d: 'D' }, 2)
    cache.prefetch(['a', 'b', 'c', 'd'])
    await settle()
    expect(cache.has('a')).toBe(false)
    expect(cache.isMiss('a')).toBe(false)
    const before = changes.count
    cache.request('a')
    expect(cache.answered('a')).toBe(false)
    await settle()
    expect(requests.at(-1)).toEqual(['a'])
    expect(cache.get('a')).toBe('A')
    expect(changes.count).toBeGreaterThan(before)
  })

  it('does not call back for an eviction alone', async () => {
    const { cache, changes } = setup({ a: 'A', b: 'B' }, 1)
    cache.prefetch(['a', 'b'])
    await settle()
    const before = changes.count
    cache.release('a')
    cache.release('b')
    expect(changes.count).toBe(before)
  })

  it('keeps a drawn thumbnail over a lookup answer that arrives later, and forgets the miss', async () => {
    const { cache } = setup({ a: 'OLD' })
    cache.request('a')
    cache.putAll({ a: 'NEW' })
    await settle()
    expect(cache.get('a')).toBe('NEW')
    cache.putAll({ z: 'Z' })
    expect(cache.isMiss('z')).toBe(false)
    expect(cache.get('z')).toBe('Z')
  })

  it('shows recently released thumbnails again without a request', async () => {
    const { cache, requests } = setup({ a: 'A', b: 'B' }, 10)
    cache.request('a')
    cache.request('b')
    await settle()
    cache.release('a')
    cache.release('b')
    cache.request('a')
    await settle()
    expect(requests).toHaveLength(1)
    expect(cache.get('a')).toBe('A')
  })

  /** A disk cache that fails its first `failing` requests and then holds `disk`. */
  function flaky(failing: number, disk: Record<string, string> = { a: 'A' }) {
    const state = { calls: 0 }
    const cache = new ThumbCache({
      load: (keys) => {
        state.calls++
        return state.calls <= failing ? Promise.reject(new Error('busy')) : Promise.resolve(Object.fromEntries(keys.filter((k) => k in disk).map((k) => [k, disk[k]])))
      },
      onChange: () => undefined,
      delayMs: 10
    })
    return { cache, state }
  }

  it('asks again by itself after a failed request, while the card stays on screen', async () => {
    const { cache, state } = flaky(1)
    cache.request('a')
    await settle()
    expect(state.calls).toBe(1)
    expect(cache.answered('a')).toBe(false)
    // No second request() call: the retry timer asks after the pause.
    await vi.advanceTimersByTimeAsync(1100)
    expect(state.calls).toBe(2)
    expect(cache.get('a')).toBe('A')
  })

  it('asks again for a card that came back on screen during the pause', async () => {
    const { cache, state } = flaky(1)
    cache.request('a')
    await settle()
    cache.release('a')
    cache.request('a')
    await vi.advanceTimersByTimeAsync(1100)
    expect(state.calls).toBe(2)
    expect(cache.get('a')).toBe('A')
  })

  it('does not retry a card that left the screen before the pause ended', async () => {
    const { cache, state } = flaky(1)
    cache.request('a')
    await settle()
    cache.release('a')
    await vi.advanceTimersByTimeAsync(5000)
    expect(state.calls).toBe(1)
    expect(cache.answered('a')).toBe(false)
  })

  it('retries a kept asset', async () => {
    const { cache, state } = flaky(1)
    cache.keep(['a'])
    await settle()
    await vi.advanceTimersByTimeAsync(1100)
    expect(state.calls).toBe(2)
    expect(cache.get('a')).toBe('A')
  })

  it('stops after the third failure: the card counts as having no thumbnail', async () => {
    const { cache, state } = flaky(10)
    cache.request('a')
    await settle()
    await vi.advanceTimersByTimeAsync(1100)
    expect(state.calls).toBe(2)
    expect(cache.answered('a')).toBe(false)
    await vi.advanceTimersByTimeAsync(2100)
    expect(state.calls).toBe(3)
    expect(cache.isMiss('a')).toBe(true)
    await vi.advanceTimersByTimeAsync(60_000)
    expect(state.calls).toBe(3)
  })

  it('stops its timers when disposed', async () => {
    const { cache, state } = flaky(1)
    cache.request('a')
    await settle()
    cache.dispose()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(state.calls).toBe(1)
  })

  it('answers after a failure that is over', async () => {
    let fail = true
    const cache = new ThumbCache({ load: (keys) => (fail ? Promise.reject(new Error('busy')) : Promise.resolve(Object.fromEntries(keys.map((k) => [k, k.toUpperCase()])))), onChange: () => undefined, delayMs: 10 })
    cache.request('a')
    await settle()
    fail = false
    await vi.advanceTimersByTimeAsync(1100)
    cache.request('a')
    await settle()
    expect(cache.get('a')).toBe('A')
  })

  it('reads saved thumbnails for an export from memory and from the disk cache without keeping them', async () => {
    const disk = Object.fromEntries(Array.from({ length: 600 }, (_, i) => [`k${i}`, `U${i}`]))
    const { cache, requests } = setup(disk, 3)
    cache.prefetch(['k0'])
    await settle()
    const before = requests.length
    const found = await cache.saved(Array.from({ length: 600 }, (_, i) => `k${i}`).concat(['k0', 'none']))
    expect(Object.keys(found)).toHaveLength(600)
    expect(found.k599).toBe('U599')
    // k0 came from memory; the other 599 keys and the unknown one were asked for in requests of bounded size.
    expect(requests.slice(before).map((r) => r.length)).toEqual([256, 256, 88])
    expect(cache.size).toBe(1)
    expect(cache.has('k599')).toBe(false)
  })
})
