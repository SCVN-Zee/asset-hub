import { env } from 'cloudflare:test'
import { describe, expect, it, vi } from 'vitest'
import { fakeRedis } from '../test/fake-redis'
import { allowRepo, api, repoPath, sha, unsizedBody, useApiHarness } from '../test/helpers'

vi.mock('./redis', async (importOriginal) => {
  const { fakeRedis: fake } = await import('../test/fake-redis')
  return { ...(await importOriginal<typeof import('./redis')>()), redis: () => fake }
})

useApiHarness()

const uuid = (n: number) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`
const event = (n: number, extra: Record<string, unknown> = {}) => ({ id: uuid(n), sha: sha(n), kind: 'view', ts: Date.now(), ...extra })
const post = (events: unknown[]) =>
  api(repoPath('game', '/events'), { method: 'POST', body: JSON.stringify({ events }), headers: { 'content-type': 'application/json' } })
const rowCount = async () => (await env.DB.prepare('SELECT COUNT(*) AS n FROM events').first<{ n: number }>())!.n

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
    const row = await env.DB.prepare('SELECT * FROM events').first<Record<string, unknown>>()
    expect(row).toMatchObject({ repo: 'supercent-io/game', kind: 'view' })
    expect(Object.keys(row!).sort()).toEqual(['id', 'kind', 'repo', 'sha', 'ts'])
  })

  it('no Redis call on ingest', async () => {
    allowRepo('game')
    await post([event(1)])
    // Only the access caches (auth, then repo permission) touch Redis.
    expect(fakeRedis.calls).toEqual(['get', 'set', 'get', 'set'])
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
    const { body, pulled } = unsizedBody(100, 1024, new TextEncoder().encode('{"events":['))
    const res = await api(repoPath('game', '/events'), { method: 'POST', body })
    expect(res.status).toBe(400)
    expect(pulled()).toBeLessThan(100)
  })
})
