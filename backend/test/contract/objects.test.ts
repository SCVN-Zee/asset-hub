import { describe, expect, it, vi } from 'vitest'
import { allowRepo, api, b2, build, db, denyRepo, repoPath, reservedBytes, SCHEMA, unsizedBody, useContract } from './harness'
import { expectErrorReplies, expectNoCors, headerNames, JSON_TYPE, snapshot, type Snapshot } from './raw'

useContract()

/** The Worker stops accepting uploads at 8 GiB of stored bytes. */
const STORAGE_CAP_BYTES = 8 * 1024 * 1024 * 1024
/** The fact kinds the Worker shares per repo, as `facts-<kind>` objects. */
const FACT_KINDS = ['metaguids', 'prefabmodels', 'prefabfx4', 'spritemeta', 'tmpfont']

const GZIP = new Uint8Array([0x1f, 0x8b, 8, 0, 1, 2, 3])
const put = (name: string, headers: Record<string, string>, body: BodyInit = GZIP, repo = 'game') =>
  api(repoPath(repo, `/cache/${name}`), { method: 'PUT', body, headers: { ...SCHEMA, ...headers } })
const get = (name: string, headers: Record<string, string> = SCHEMA, repo = 'game') =>
  api(repoPath(repo, `/cache/${name}`), { headers })
const create = async (name = 'catalog') => (await put(name, { 'if-none-match': '*' })).headers.get('etag')!
const setReserved = async (bytes: number) => void (await (await db()).prepare('UPDATE storage SET reserved_bytes = ?').bind(bytes).run())

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
    // 60 MB offered in 1 MB chunks, limit 20 MB: the Worker must stop reading well before the end.
    const { body, pulled } = unsizedBody(60, 1024 * 1024, GZIP.subarray(0, 2), 2)
    const res = await put('catalog', { 'if-none-match': '*' }, body)
    expect(res.status).toBe(413)
    expect(pulled()).toBeLessThan(60)
    expect(b2.keys()).toEqual([])
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
    expect(b2.keys()).toEqual([expect.stringMatching(/^repos\/supercent-io\/game\/s1\/catalog\/[0-9a-f-]{36}\.json\.gz$/)])
  })

  it('PUT needs push', async () => {
    allowRepo('game', false)
    const res = await put('catalog', { 'if-none-match': '*' })
    expect(res.status).toBe(403)
    expect(await res.json()).toEqual({ error: 'readOnly' })
    expect(b2.keys()).toEqual([])
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
    b2.calls = []
    const res = await get('catalog', { ...SCHEMA, 'if-none-match': etag })
    expect(res.status).toBe(304)
    expect(b2.calls).toEqual([])
  })

  it('a replace deletes the old version and keeps one stored object', async () => {
    allowRepo('game', true)
    const first = await create()
    const [oldKey] = b2.keys()
    expect((await put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 9, 9]))).status).toBe(200)
    expect(b2.versionsOf(oldKey)).toEqual([])
    expect(b2.keys()).toHaveLength(1)
    expect(await reservedBytes()).toBe(4)
  })

  it('a lost race deletes the new version and releases its bytes', async () => {
    allowRepo('game', true)
    const first = await create()
    let release!: () => void
    const gate = new Promise<void>((resolve) => (release = resolve))
    b2.hold = async (call) => {
      if (call.method === 'PUT') await gate
    }
    const a = put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 1, 1, 1]))
    const b = put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 2, 2, 2, 2]))
    await vi.waitFor(() => expect(b2.callsTo('PUT')).toHaveLength(3))
    release()
    const statuses = (await Promise.all([a, b])).map((r) => r.status).sort()
    expect(statuses).toEqual([200, 412])
    expect(b2.keys()).toHaveLength(1)
    expect(await reservedBytes()).toBe(b2.totalBytes())
    expect(await reservedBytes()).toBeGreaterThanOrEqual(5)
    expect(await reservedBytes()).toBeLessThanOrEqual(6)
  })

  it('a missing body re-reads the head once', async () => {
    allowRepo('game', true)
    const first = await create()
    let replaced = false
    b2.hold = async (call) => {
      if (call.method !== 'GET' || replaced) return
      replaced = true
      // A writer replaces the object after this reader read the head.
      b2.hold = null
      expect((await put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 42]))).status).toBe(200)
    }
    const res = await get('catalog')
    expect(res.status).toBe(200)
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(new Uint8Array([0x1f, 0x8b, 42]))
    expect(b2.callsTo('GET')).toHaveLength(2)
  })

  it('503 when the body is missing twice', async () => {
    allowRepo('game', true)
    await create()
    b2.failWith = (call) => (call.method === 'GET' ? 404 : null)
    const res = await get('catalog')
    expect(res.status).toBe(503)
    expect(b2.callsTo('GET')).toHaveLength(2)
  })

  // The TypeScript Worker answers 500 here. The Rust Worker reports an unusable store as 503.
  describe.runIf(build === 'rust')('a B2 error on a cache object read', () => {
    for (const status of [403, 500]) {
      it(`${status} gives 503 storageUnavailable`, async () => {
        allowRepo('game', true)
        await create()
        b2.failWith = (call) => (call.method === 'GET' ? status : null)
        const res = await get('catalog')
        expect(res.status).toBe(503)
        expect(await res.json()).toEqual({ error: 'storageUnavailable' })
      })
    }
  })

  it('507 at the storage cap and nothing is uploaded', async () => {
    allowRepo('game', true)
    await setReserved(STORAGE_CAP_BYTES - 5)
    const res = await put('catalog', { 'if-none-match': '*' })
    expect(res.status).toBe(507)
    expect(await res.json()).toEqual({ error: 'storageFull' })
    expect(b2.calls).toEqual([])
    expect(await reservedBytes()).toBe(STORAGE_CAP_BYTES - 5)
  })

  it('two writers near the cap: only one passes', async () => {
    allowRepo('game', true)
    await setReserved(STORAGE_CAP_BYTES - 10)
    const results = await Promise.all([put('catalog', { 'if-none-match': '*' }), put('stats', { 'if-none-match': '*' })])
    expect(results.map((r) => r.status).sort()).toEqual([200, 507])
    expect(await reservedBytes()).toBe(STORAGE_CAP_BYTES - 10 + GZIP.length)
  })

  it('a failed delete keeps its bytes charged', async () => {
    allowRepo('game', true)
    const first = await create()
    b2.failWith = (call) => (call.method === 'DELETE' ? 500 : null)
    const bigger = new Uint8Array([0x1f, 0x8b, 1, 2, 3, 4, 5, 6])
    expect((await put('catalog', { 'if-match': first }, bigger)).status).toBe(200)
    expect(await reservedBytes()).toBe(GZIP.length + bigger.length)
  })

  it('an upload refused by B2 releases its bytes, a lost one does not', async () => {
    allowRepo('game', true)
    b2.failWith = (call) => (call.method === 'PUT' ? 503 : null)
    expect((await put('catalog', { 'if-none-match': '*' })).status).toBe(500)
    expect(await reservedBytes()).toBe(0)

    b2.failWith = null
    // A hook that throws drops the connection, as a lost network would.
    b2.hold = async (call) => {
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
    expect(b2.calls.length).toBeGreaterThan(0)
    expect(b2.badSignatures).toEqual([])
    const deletes = b2.callsTo('DELETE')
    expect(deletes.length).toBeGreaterThan(0)
    expect(deletes.every((c) => c.versionId)).toBe(true)
  })

  it('a delete of a version B2 no longer has releases nothing', async () => {
    allowRepo('game', true)
    const first = await create()
    const [oldKey] = b2.keys()
    // The old version vanishes (for example through a lifecycle rule) before the replace deletes it.
    b2.hold = async (call) => {
      if (call.method === 'DELETE') b2.dropAll(oldKey)
    }
    expect((await put('catalog', { 'if-match': first }, new Uint8Array([0x1f, 0x8b, 9, 9]))).status).toBe(200)
    expect(await reservedBytes()).toBe(GZIP.length + 4)
  })
})

describe('cache objects raw bytes and headers', () => {
  it('a create reply carries the etag in the body and in the header', async () => {
    allowRepo('game', true)
    const res = await put('catalog', { 'if-none-match': '*' })
    const text = await res.text()
    const etag = res.headers.get('etag')!
    expect(etag).toMatch(/^"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"$/)
    expect(text).toBe(`{"etag":${JSON.stringify(etag)}}`)
    expect(res.headers.get('content-type')).toBe(JSON_TYPE)
    expect(res.headers.get('cache-control')).toBe('no-transform')
    expect(headerNames(res)).toEqual(['cache-control', 'content-type', 'etag'])
    expectNoCors(res)
  })

  it('a read reply has exactly the etag, gzip type and no-store headers', async () => {
    allowRepo('game', true)
    const etag = await create()
    const res = await get('catalog')
    expect(res.headers.get('etag')).toBe(etag)
    expect(res.headers.get('content-type')).toBe('application/gzip')
    expect(res.headers.get('cache-control')).toBe('private, no-store')
    expect(headerNames(res)).toEqual(['cache-control', 'content-type', 'etag'])
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(GZIP)
    expectNoCors(res)
  })

  it('a 304 carries only the etag', async () => {
    allowRepo('game', true)
    const etag = await create()
    const res = await get('catalog', { ...SCHEMA, 'if-none-match': etag })
    expect(res.status).toBe(304)
    expect(await res.text()).toBe('')
    expect(headerNames(res)).toEqual(['etag'])
    expectNoCors(res)
  })

  it('error replies are exact JSON', async () => {
    allowRepo('game', true)
    const cases: Array<[Snapshot, number, string]> = [
      [await snapshot(await get('stats')), 404, '{"error":"notFound"}'],
      [await snapshot(await put('catalog', {})), 428, '{"error":"preconditionRequired"}'],
      [await snapshot(await put('catalog', { 'if-none-match': '*' }, new Uint8Array([1, 2, 3]))), 400, '{"error":"notGzip"}'],
      [await snapshot(await get('catalog', {})), 400, '{"error":"badSchema"}']
    ]
    await create()
    cases.push([await snapshot(await put('catalog', { 'if-none-match': '*' })), 412, '{"error":"preconditionFailed"}'])
    await setReserved(STORAGE_CAP_BYTES)
    cases.push([await snapshot(await put('stats', { 'if-none-match': '*' })), 507, '{"error":"storageFull"}'])
    expectErrorReplies(cases)
  })
})
