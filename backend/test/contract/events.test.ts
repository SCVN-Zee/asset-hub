import { describe, expect, it } from 'vitest'
import { allowRepo, api, db, repoPath, sha, unsizedBody, upstash, useContract } from './harness'
import { expectNoCors, JSON_TYPE } from './raw'

useContract()

const uuid = (n: number) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`
const event = (n: number, extra: Record<string, unknown> = {}) => ({ id: uuid(n), sha: sha(n), kind: 'view', ts: Date.now(), ...extra })
const post = (events: unknown[]) =>
  api(repoPath('game', '/events'), { method: 'POST', body: JSON.stringify({ events }), headers: { 'content-type': 'application/json' } })
const rowCount = async () => (await (await db()).prepare('SELECT COUNT(*) AS n FROM events').first<{ n: number }>())!.n

describe('POST /events', () => {
  it('dedups by id', async () => {
    allowRepo('game')
    const batch = [event(1), event(2, { kind: 'send' })]
    expect(await (await post(batch)).json()).toEqual({ accepted: 2, rejected: [] })
    expect(await (await post(batch)).json()).toEqual({ accepted: 2, rejected: [] })
    expect(await rowCount()).toBe(2)
  })

  it('rejects 101 events', async () => {
    allowRepo('game')
    const res = await post(Array.from({ length: 101 }, (_, i) => event(i + 1)))
    expect(res.status).toBe(400)
    expect(await rowCount()).toBe(0)
  })

  it('rejects an empty batch and a body over 32 KB', async () => {
    allowRepo('game')
    expect((await post([])).status).toBe(400)
    const big = await post(Array.from({ length: 100 }, (_, i) => event(i + 1, { pad: 'x'.repeat(400) })))
    expect(big.status).toBe(400)
  })

  it('reports bad sha in rejected', async () => {
    allowRepo('game')
    const res = await post([event(1), event(2, { sha: 'not-a-sha' })])
    expect(await res.json()).toEqual({ accepted: 1, rejected: [uuid(2)] })
    expect(await rowCount()).toBe(1)
  })

  it('rejects stale, future, and unknown-kind events without failing the batch', async () => {
    allowRepo('game')
    const day = 24 * 60 * 60 * 1000
    const res = await post([
      event(1),
      event(2, { ts: Date.now() - 8 * day }),
      event(3, { ts: Date.now() + 10 * 60 * 1000 }),
      event(4, { kind: 'delete' }),
      event(5, { id: 'not-a-uuid' })
    ])
    expect(await res.json()).toEqual({ accepted: 1, rejected: [uuid(2), uuid(3), uuid(4), 'not-a-uuid'] })
  })

  it('stores the repo in lower case and no login', async () => {
    allowRepo('game')
    await post([event(1)])
    const row = await (await db()).prepare('SELECT * FROM events').first<Record<string, unknown>>()
    expect(row).toMatchObject({ repo: 'supercent-io/game', kind: 'view' })
    expect(Object.keys(row!).sort()).toEqual(['id', 'kind', 'repo', 'sha', 'ts'])
  })

  it('no Redis call on ingest', async () => {
    allowRepo('game')
    await post([event(1)])
    // Only the access caches (auth, then repo permission) touch Redis.
    expect(upstash.commandNames()).toEqual(['GET', 'SET', 'GET', 'SET'])
  })

  it('same id in two repos stores both', async () => {
    allowRepo('game')
    allowRepo('other')
    const batch = [event(1)]
    const body = JSON.stringify({ events: batch })
    expect((await api(repoPath('game', '/events'), { method: 'POST', body })).status).toBe(200)
    expect((await api(repoPath('other', '/events'), { method: 'POST', body })).status).toBe(200)
    expect(await rowCount()).toBe(2)
  })

  it('rejects oversized body without content-length', async () => {
    allowRepo('game')
    // 2000 chunks of 1 KB with a pause each: the limit is 32 KB, so the Worker must stop reading early.
    const { body, pulled } = unsizedBody(2000, 1024, new TextEncoder().encode('{"events":['), 1)
    const res = await api(repoPath('game', '/events'), { method: 'POST', body })
    expect(res.status).toBe(400)
    expect(pulled()).toBeLessThan(2000)
  })
})

describe('events raw bytes and headers', () => {
  it('success and error bodies are exact', async () => {
    allowRepo('game')
    const ok = await post([event(1)])
    expect(await ok.text()).toBe('{"accepted":1,"rejected":[]}')
    expect(ok.headers.get('content-type')).toBe(JSON_TYPE)
    expectNoCors(ok)

    const partial = await post([event(2), event(3, { sha: 'bad' })])
    expect(await partial.text()).toBe(`{"accepted":1,"rejected":["${uuid(3)}"]}`)

    const bad = await post([])
    expect(bad.status).toBe(400)
    expect(await bad.text()).toBe('{"error":"badRequest"}')
    expect(bad.headers.get('content-type')).toBe(JSON_TYPE)
    expectNoCors(bad)
  })
})
