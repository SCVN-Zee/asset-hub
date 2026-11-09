import { env } from 'cloudflare:test'
import { describe, expect, it, vi } from 'vitest'
import { fakeRedis } from '../test/fake-redis'
import { allowRepo, api, repoPath, sha, useApiHarness } from '../test/helpers'
import { monthlyRedisCommands, rebuildPopular } from './popular'

vi.mock('./redis', async (importOriginal) => {
  const { fakeRedis: fake } = await import('../test/fake-redis')
  return { ...(await importOriginal<typeof import('./redis')>()), redis: () => fake }
})

useApiHarness()

const DAY = 24 * 60 * 60 * 1000
let nextId = 0

async function addEvent(repo: string, shaValue: string, kind: 'view' | 'send', ageDays: number): Promise<void> {
  nextId += 1
  await env.DB.prepare('INSERT INTO events (id, repo, sha, kind, ts) VALUES (?, ?, ?, ?, ?)')
    .bind(`e${nextId}`, repo, shaValue, kind, Date.now() - ageDays * DAY)
    .run()
}

const rankingOf = (repo: string) => fakeRedis.replaced.find((r) => r.key === `pop:${repo}`)!.members

describe('popularity rebuild', () => {
  it('send outweighs view', async () => {
    await addEvent('o/r', sha(1), 'send', 0)
    await addEvent('o/r', sha(2), 'view', 0)
    await addEvent('o/r', sha(2), 'view', 0)
    await rebuildPopular(env)
    const members = rankingOf('o/r')
    expect(members.map((m) => m.member)).toEqual([sha(1), sha(2)])
    expect(members[0].score).toBeCloseTo(3, 3)
    expect(members[1].score).toBeCloseTo(2, 3)
  })

  it('older events decay', async () => {
    await addEvent('o/r', sha(1), 'view', 14)
    await addEvent('o/r', sha(2), 'view', 0)
    await rebuildPopular(env)
    const members = rankingOf('o/r')
    expect(members.map((m) => m.member)).toEqual([sha(2), sha(1)])
    expect(members[1].score).toBeCloseTo(0.5, 2)
  })

  it('keeps top 200', async () => {
    for (let i = 1; i <= 250; i++) await addEvent('o/r', sha(i), 'view', 0)
    await addEvent('o/r', sha(250), 'send', 0)
    await rebuildPopular(env)
    const members = rankingOf('o/r')
    expect(members).toHaveLength(200)
    expect(members[0].member).toBe(sha(250))
  })

  it('rebuild refreshes 60 repos in one pipeline', async () => {
    for (let i = 0; i < 60; i++) await addEvent(`o/r${i}`, sha(i + 1), 'view', 0)
    await rebuildPopular(env)
    expect(fakeRedis.batches).toEqual([60])
    expect(fakeRedis.replaced).toHaveLength(60)
  })

  it('does not call Redis when there are no events', async () => {
    await rebuildPopular(env)
    expect(fakeRedis.calls).toEqual([])
  })

  it('ignores events older than 60 days and deletes events older than 90', async () => {
    await addEvent('o/r', sha(1), 'view', 70)
    await addEvent('o/r', sha(2), 'view', 95)
    await addEvent('o/r', sha(3), 'view', 1)
    await rebuildPopular(env)
    expect(rankingOf('o/r').map((m) => m.member)).toEqual([sha(3)])
    const left = await env.DB.prepare('SELECT sha FROM events ORDER BY sha').all<{ sha: string }>()
    expect(left.results.map((r) => r.sha)).toEqual([sha(1), sha(3)])
  })

  it('stays inside the Upstash free tier with 51 repos', () => {
    // 5 commands x 51 repos x 8 rebuilds a day x 30 days.
    expect(monthlyRedisCommands(51)).toBe(61_200)
    expect(monthlyRedisCommands(51)).toBeLessThan(500_000)
  })
})

describe('GET /popular', () => {
  it('returns ranked shas with builtAt', async () => {
    allowRepo('game')
    await addEvent('supercent-io/game', sha(1), 'send', 0)
    await addEvent('supercent-io/game', sha(2), 'view', 0)
    await rebuildPopular(env)
    const res = await api(repoPath('game', '/popular'))
    expect(res.status).toBe(200)
    const body = (await res.json()) as { items: Array<{ sha: string; score: number }>; builtAt: number }
    expect(body.items.map((i) => i.sha)).toEqual([sha(1), sha(2)])
    expect(typeof body.builtAt).toBe('number')
    expect(fakeRedis.calls.filter((c) => c === 'ranking')).toHaveLength(1)
  })

  it('returns an empty list for a repo without a ranking', async () => {
    allowRepo('game')
    expect(await (await api(repoPath('game', '/popular'))).json()).toEqual({ items: [], builtAt: null })
  })

  it('503 when Redis is down', async () => {
    allowRepo('game')
    fakeRedis.down = true
    const res = await api(repoPath('game', '/popular'))
    expect(res.status).toBe(503)
    expect(await res.json()).toEqual({ error: 'redisDown' })
  })
})
