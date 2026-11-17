import { env } from 'cloudflare:test'
import { describe, expect, it, vi } from 'vitest'
import { allowRepo, api, breakD1, denyRepo, fakeB2, repoPath, reservedBytes, SCHEMA, unsizedBody, useApiHarness } from '../test/helpers'
import { STORAGE_CAP_BYTES } from './storage'
import { FACT_KINDS } from './kinds'

vi.mock('./redis', async (importOriginal) => {
  const { fakeRedis: fake } = await import('../test/fake-redis')
  return { ...(await importOriginal<typeof import('./redis')>()), redis: () => fake }
})

useApiHarness()

const GZIP = new Uint8Array([0x1f, 0x8b, 8, 0, 1, 2, 3])
const put = (name: string, headers: Record<string, string>, body: BodyInit = GZIP, repo = 'game') =>
  api(repoPath(repo, `/cache/${name}`), { method: 'PUT', body, headers: { ...SCHEMA, ...headers } })
const get = (name: string, headers: Record<string, string> = SCHEMA, repo = 'game') =>
  api(repoPath(repo, `/cache/${name}`), { headers })
const create = async (name = 'catalog') => (await put(name, { 'if-none-match': '*' })).headers.get('etag')!

describe('cache objects', () => {
  it('stores on create and returns the same bytes with an ETag', async () => {
    allowRepo('game', true)
    const etag = await create()
    expect(etag).toBeTruthy()
    const res = await get('catalog')
    expect(res.status).toBe(200)
    expect(res.headers.get('content-type')).toBe('application/gzip')
    expect(res.headers.get('etag')).toBe(etag)
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(GZIP)
  })

  it('304 on matching etag', async () => {
    allowRepo('game', true)
    const etag = await create()
    const res = await get('catalog', { ...SCHEMA, 'if-none-match': etag })
    expect(res.status).toBe(304)
    expect(res.headers.get('etag')).toBe(etag)
    const changed = await get('catalog', { ...SCHEMA, 'if-none-match': '"other"' })
    expect(changed.status).toBe(200)
  })

  it('404 for a missing object', async () => {
    allowRepo('game')
    const res = await get('stats')
    expect(res.status).toBe(404)
    expect(await res.json()).toEqual({ error: 'notFound' })
  })

  it('412 on stale etag', async () => {
    allowRepo('game', true)
    const first = await create()
    const winner = await put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 9]))
    expect(winner.status).toBe(200)
    const loser = await put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 10]))
    expect(loser.status).toBe(412)
    expect(await loser.json()).toEqual({ error: 'preconditionFailed' })
    expect(new Uint8Array(await (await get('catalog')).arrayBuffer())).toEqual(new Uint8Array([0x1f, 0x8b, 9]))
  })

  it('412 when creating over an existing object', async () => {
    allowRepo('game', true)
    await create()
    expect((await put('catalog', { 'if-none-match': '*' })).status).toBe(412)
  })

  it('428 without a precondition header', async () => {
    allowRepo('game', true)
    expect((await put('catalog', {})).status).toBe(428)
  })

  it('If-Match * is rejected', async () => {
    allowRepo('game', true)
    await create()
    const res = await put('catalog', { 'if-match': '*' })
    expect(res.status).toBe(428)
    expect(await res.json()).toEqual({ error: 'preconditionRequired' })
    expect((await put('stats', { 'if-match': '*' })).status).toBe(428)
  })

  it('rejects oversized body without content-length', async () => {
    allowRepo('game', true)
    const { body, pulled } = unsizedBody(30, 1024 * 1024, GZIP.subarray(0, 2)) // 30 MB, limit 20 MB
    const res = await put('catalog', { 'if-none-match': '*' }, body)
    expect(res.status).toBe(413)
    expect(pulled()).toBeLessThan(30)
    expect(fakeB2.keys()).toEqual([])
  })

  it('rejects non-gzip', async () => {
    allowRepo('game', true)
    const res = await put('catalog', { 'if-none-match': '*' }, new Uint8Array([1, 2, 3]))
    expect(res.status).toBe(400)
    expect(await res.json()).toEqual({ error: 'notGzip' })
  })

  it('rejects a body over 20 MB', async () => {
    allowRepo('game', true)
    const big = new Uint8Array(20 * 1024 * 1024 + 1)
    big[0] = 0x1f
    big[1] = 0x8b
    expect((await put('catalog', { 'if-none-match': '*' }, big)).status).toBe(413)
  })

  it('accepts facts-prefabfx4', async () => {
    allowRepo('game', true)
    expect((await put('facts-prefabfx4', { 'if-none-match': '*' })).status).toBe(200)
  })

  it('accepts every fact kind', async () => {
    allowRepo('game', true)
    for (const kind of FACT_KINDS) expect((await put(`facts-${kind}`, { 'if-none-match': '*' })).status).toBe(200)
  })

  it('rejects unknown name', async () => {
    allowRepo('game', true)
    for (const name of ['facts-other', 'facts', 'secrets', 'catalog.json']) {
      expect((await put(name, { 'if-none-match': '*' })).status).toBe(404)
      expect((await get(name)).status).toBe(404)
    }
  })

  it('400 without a valid schema header', async () => {
    allowRepo('game', true)
    expect((await get('catalog', {})).status).toBe(400)
    expect((await get('catalog', { 'x-asset-hub-schema': 'abc' })).status).toBe(400)
    expect((await put('catalog', { 'if-none-match': '*', 'x-asset-hub-schema': '1.5' })).status).toBe(400)
  })

  it('schema namespaces keys', async () => {
    allowRepo('game', true)
    await create()
    const other = await get('catalog', { 'x-asset-hub-schema': '2' })
    expect(other.status).toBe(404)
    expect(fakeB2.keys()).toEqual([expect.stringMatching(/^repos\/supercent-io\/game\/s1\/catalog\/[0-9a-f-]{36}\.json\.gz$/)])
  })

  it('PUT needs push', async () => {
    allowRepo('game', false)
    const res = await put('catalog', { 'if-none-match': '*' })
    expect(res.status).toBe(403)
    expect(await res.json()).toEqual({ error: 'readOnly' })
    expect(fakeB2.keys()).toEqual([])
  })

  it('repo B object not readable with repo A access', async () => {
    allowRepo('a', true)
    denyRepo('b')
    await put('catalog', { 'if-none-match': '*' }, GZIP, 'a')
    expect((await get('catalog', SCHEMA, 'a')).status).toBe(200)
    const denied = await get('catalog', SCHEMA, 'b')
    expect(denied.status).toBe(403)
    expect(await denied.json()).toEqual({ error: 'noAccess' })
  })

  it('304 on a matching etag makes no B2 call', async () => {
    allowRepo('game', true)
    const etag = await create()
    fakeB2.calls = []
    const res = await get('catalog', { ...SCHEMA, 'if-none-match': etag })
    expect(res.status).toBe(304)
    expect(fakeB2.calls).toEqual([])
  })

  it('a replace deletes the old version and keeps one stored object', async () => {
    allowRepo('game', true)
    const first = await create()
    const [oldKey] = fakeB2.keys()
    expect((await put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 9, 9]))).status).toBe(200)
    expect(fakeB2.versionsOf(oldKey)).toEqual([])
    expect(fakeB2.keys()).toHaveLength(1)
    expect(await reservedBytes()).toBe(4)
  })

  it('a lost race deletes the new version and releases its bytes', async () => {
    allowRepo('game', true)
    const first = await create()
    let release!: () => void
    const gate = new Promise<void>((resolve) => (release = resolve))
    fakeB2.hold = async (call) => {
      if (call.method === 'PUT') await gate
    }
    const a = put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 1, 1, 1]))
    const b = put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 2, 2, 2, 2]))
    await vi.waitFor(() => expect(fakeB2.callsTo('PUT')).toHaveLength(3))
    release()
    const statuses = (await Promise.all([a, b])).map((r) => r.status).sort()
    expect(statuses).toEqual([200, 412])
    expect(fakeB2.keys()).toHaveLength(1)
    expect(await reservedBytes()).toBe(fakeB2.totalBytes())
    expect(await reservedBytes()).toBeGreaterThanOrEqual(5)
    expect(await reservedBytes()).toBeLessThanOrEqual(6)
  })

  it('a missing body re-reads the head once', async () => {
    allowRepo('game', true)
    const first = await create()
    let replaced = false
    fakeB2.hold = async (call) => {
      if (call.method !== 'GET' || replaced) return
      replaced = true
      // A writer replaces the object after this reader read the head.
      fakeB2.hold = null
      expect((await put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 42]))).status).toBe(200)
    }
    const res = await get('catalog')
    expect(res.status).toBe(200)
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(new Uint8Array([0x1f, 0x8b, 42]))
    expect(fakeB2.callsTo('GET')).toHaveLength(2)
  })

  it('503 when the body is missing twice', async () => {
    allowRepo('game', true)
    await create()
    fakeB2.failWith = (call) => (call.method === 'GET' ? 404 : null)
    const res = await get('catalog')
    expect(res.status).toBe(503)
    expect(fakeB2.callsTo('GET')).toHaveLength(2)
  })

  it('507 at the storage cap and nothing is uploaded', async () => {
    allowRepo('game', true)
    await env.DB.prepare('UPDATE storage SET reserved_bytes = ?').bind(STORAGE_CAP_BYTES - 5).run()
    const res = await put('catalog', { 'if-none-match': '*' })
    expect(res.status).toBe(507)
    expect(await res.json()).toEqual({ error: 'storageFull' })
    expect(fakeB2.calls).toEqual([])
    expect(await reservedBytes()).toBe(STORAGE_CAP_BYTES - 5)
  })

  it('two writers near the cap: only one passes', async () => {
    allowRepo('game', true)
    await env.DB.prepare('UPDATE storage SET reserved_bytes = ?').bind(STORAGE_CAP_BYTES - 10).run()
    const results = await Promise.all([put('catalog', { 'if-none-match': '*' }), put('stats', { 'if-none-match': '*' })])
    expect(results.map((r) => r.status).sort()).toEqual([200, 507])
    expect(await reservedBytes()).toBe(STORAGE_CAP_BYTES - 10 + GZIP.length)
  })

  it('a failed delete keeps its bytes charged', async () => {
    allowRepo('game', true)
    const first = await create()
    fakeB2.failWith = (call) => (call.method === 'DELETE' ? 500 : null)
    const bigger = new Uint8Array([0x1f, 0x8b, 1, 2, 3, 4, 5, 6])
    expect((await put('catalog', { 'if-match': first }, bigger)).status).toBe(200)
    expect(await reservedBytes()).toBe(GZIP.length + bigger.length)
  })

  it('an upload refused by B2 releases its bytes, a lost one does not', async () => {
    allowRepo('game', true)
    fakeB2.failWith = (call) => (call.method === 'PUT' ? 503 : null)
    expect((await put('catalog', { 'if-none-match': '*' })).status).toBe(500)
    expect(await reservedBytes()).toBe(0)

    fakeB2.failWith = null
    fakeB2.hold = async (call) => {
      if (call.method === 'PUT') throw new TypeError('network lost')
    }
    expect((await put('catalog', { 'if-none-match': '*' })).status).toBe(500)
    expect(await reservedBytes()).toBe(GZIP.length)
  })

  it('every B2 request is signed and every delete names a version', async () => {
    allowRepo('game', true)
    const first = await create()
    await put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 9]))
    await get('catalog')
    expect(fakeB2.unsigned).toBe(0)
    const deletes = fakeB2.callsTo('DELETE')
    expect(deletes.length).toBeGreaterThan(0)
    expect(deletes.every((c) => c.versionId)).toBe(true)
  })

  const COMMIT = /^INSERT INTO objects/
  const REREAD = /^SELECT 1 AS present FROM objects/

  it('a commit that threw but landed keeps the live version and its bytes', async () => {
    allowRepo('game', true)
    breakD1({ sql: COMMIT, mode: 'after' })
    expect((await put('catalog', { 'if-none-match': '*' })).status).toBe(200)
    expect(fakeB2.keys()).toHaveLength(1)
    expect(await reservedBytes()).toBe(GZIP.length)
    expect((await get('catalog')).status).toBe(200)
  })

  it('a commit that threw and did not land deletes the new version and releases its bytes', async () => {
    allowRepo('game', true)
    breakD1({ sql: COMMIT, mode: 'before' })
    expect((await put('catalog', { 'if-none-match': '*' })).status).toBe(500)
    expect(fakeB2.keys()).toEqual([])
    expect(await reservedBytes()).toBe(0)
  })

  it('when the commit outcome cannot be read, nothing is deleted and the bytes stay charged', async () => {
    allowRepo('game', true)
    breakD1({ sql: COMMIT, mode: 'after' }, { sql: REREAD, mode: 'before' })
    expect((await put('catalog', { 'if-none-match': '*' })).status).toBe(500)
    expect(fakeB2.keys()).toHaveLength(1)
    expect(fakeB2.callsTo('DELETE')).toEqual([])
    expect(await reservedBytes()).toBe(GZIP.length)
  })

  it('a delete of a version B2 no longer has releases nothing', async () => {
    allowRepo('game', true)
    const first = await create()
    const [oldKey] = fakeB2.keys()
    // The old version vanishes (for example through a lifecycle rule) before the replace deletes it.
    fakeB2.hold = async (call) => {
      if (call.method === 'DELETE') fakeB2.dropAll(oldKey)
    }
    expect((await put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 9, 9]))).status).toBe(200)
    expect(await reservedBytes()).toBe(GZIP.length + 4)
  })
})
