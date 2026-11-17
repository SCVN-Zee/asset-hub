import { describe, expect, it } from 'vitest'
import { allowRepo, api, db, OWNER, repoPath, runScheduled, sha, upstash, useContract } from './harness'
import { expectNoCors, JSON_TYPE } from './raw'

useContract()

const DAY = 24 * 60 * 60 * 1000
let nextId = 0

async function addEvent(repo: string, shaValue: string, kind: 'view' | 'send', ageDays: number): Promise<void> {
  nextId += 1
  await (await db())
    .prepare('INSERT INTO events (id, repo, sha, kind, ts) VALUES (?, ?, ?, ?, ?)')
    .bind(`e${nextId}`, repo, shaValue, kind, Date.now() - ageDays * DAY)
    .run()
}

const rankingOf = (repo: string) => upstash.zset(`pop:${repo}`)
const rankingRequests = () => upstash.requests.filter((r) => r.commands.some((c) => String(c[0]).toUpperCase() === 'ZRANGE'))

describe('popularity rebuild', () => {
  it('send outweighs view', async () => {
    await addEvent('o/r', sha(1), 'send', 0)
    await addEvent('o/r', sha(2), 'view', 0)
    await addEvent('o/r', sha(2), 'view', 0)
    await runScheduled()
    const members = rankingOf('o/r')
    expect(members.map((m) => m.member)).toEqual([sha(1), sha(2)])
    expect(members[0].score).toBeCloseTo(3, 3)
    expect(members[1].score).toBeCloseTo(2, 3)
  })

  it('older events decay', async () => {
    await addEvent('o/r', sha(1), 'view', 14)
    await addEvent('o/r', sha(2), 'view', 0)
    await runScheduled()
    const members = rankingOf('o/r')
    expect(members.map((m) => m.member)).toEqual([sha(2), sha(1)])
    expect(members[1].score).toBeCloseTo(0.5, 2)
  })

  it('keeps top 200', async () => {
    for (let i = 1; i <= 250; i++) await addEvent('o/r', sha(i), 'view', 0)
    await addEvent('o/r', sha(250), 'send', 0)
    await runScheduled()
    const members = rankingOf('o/r')
    expect(members).toHaveLength(200)
    expect(members[0].member).toBe(sha(250))
  })

  it('rebuild refreshes 60 repos in one pipeline', async () => {
    for (let i = 0; i < 60; i++) await addEvent(`o/r${i}`, sha(i + 1), 'view', 0)
    await runScheduled()
    expect(upstash.requests).toHaveLength(1)
    expect(upstash.requests[0].path).toBe('/pipeline')
    // Five commands for each repo: DEL, ZADD, RENAME, EXPIRE, SET.
    expect(upstash.requests[0].commands).toHaveLength(300)
    expect(upstash.keys().filter((k) => /^pop:o\/r\d+$/.test(k))).toHaveLength(60)
  })

  it('does not call Redis when there are no events', async () => {
    await runScheduled()
    expect(upstash.requests).toEqual([])
  })

  it('ignores events older than 60 days and deletes events older than 90', async () => {
    await addEvent('o/r', sha(1), 'view', 70)
    await addEvent('o/r', sha(2), 'view', 95)
    await addEvent('o/r', sha(3), 'view', 1)
    await runScheduled()
    expect(rankingOf('o/r').map((m) => m.member)).toEqual([sha(3)])
    const left = await (await db()).prepare('SELECT sha FROM events ORDER BY sha').all<{ sha: string }>()
    expect(left.results.map((r) => r.sha)).toEqual([sha(1), sha(3)])
  })
})

describe('GET /popular', () => {
  it('returns ranked shas with builtAt', async () => {
    allowRepo('game')
    await addEvent('supercent-io/game', sha(1), 'send', 0)
    await addEvent('supercent-io/game', sha(2), 'view', 0)
    await runScheduled()
    const res = await api(repoPath('game', '/popular'))
    expect(res.status).toBe(200)
    const body = (await res.json()) as { items: Array<{ sha: string; score: number }>; builtAt: number }
    expect(body.items.map((i) => i.sha)).toEqual([sha(1), sha(2)])
    expect(typeof body.builtAt).toBe('number')
    expect(rankingRequests()).toHaveLength(1)
  })

  it('returns an empty list for a repo without a ranking', async () => {
    allowRepo('game')
    expect(await (await api(repoPath('game', '/popular'))).json()).toEqual({ items: [], builtAt: null })
  })

  it('503 when Redis is down', async () => {
    allowRepo('game')
    upstash.down = true
    const res = await api(repoPath('game', '/popular'))
    expect(res.status).toBe(503)
    expect(await res.json()).toEqual({ error: 'redisDown' })
  })
})

describe('popular raw bytes and headers', () => {
  const key = `pop:${OWNER}/game`

  it('an integer score has no decimal point and a fractional score keeps it', async () => {
    allowRepo('game')
    upstash.seedZset(key, [
      { member: sha(1), score: 3 },
      { member: sha(2), score: 0.5 }
    ])
    upstash.seedString(`${key}:at`, '1700000000000')
    const res = await api(repoPath('game', '/popular'))
    expect(await res.text()).toBe(`{"items":[{"sha":"${sha(1)}","score":3},{"sha":"${sha(2)}","score":0.5}],"builtAt":1700000000000}`)
    expect(res.headers.get('content-type')).toBe(JSON_TYPE)
    expectNoCors(res)
  })

  it('an empty ranking and the Redis-down error are exact', async () => {
    allowRepo('game')
    expect(await (await api(repoPath('game', '/popular'))).text()).toBe('{"items":[],"builtAt":null}')
    upstash.down = true
    const down = await api(repoPath('game', '/popular'))
    expect(await down.text()).toBe('{"error":"redisDown"}')
    expect(down.headers.get('content-type')).toBe(JSON_TYPE)
  })
})
