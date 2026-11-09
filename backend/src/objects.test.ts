import { env } from 'cloudflare:test'
import { describe, expect, it, vi } from 'vitest'
import { allowRepo, api, denyRepo, repoPath, SCHEMA, unsizedBody, useApiHarness } from '../test/helpers'
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
    expect((await env.CACHE.list()).objects).toHaveLength(0)
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

  it('accepts facts-prefabfx3', async () => {
    allowRepo('game', true)
    expect((await put('facts-prefabfx3', { 'if-none-match': '*' })).status).toBe(200)
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
    const keys = (await env.CACHE.list()).objects.map((o) => o.key)
    expect(keys).toEqual(['repos/supercent-io/game/s1/catalog.json.gz'])
  })

  it('PUT needs push', async () => {
    allowRepo('game', false)
    const res = await put('catalog', { 'if-none-match': '*' })
    expect(res.status).toBe(403)
    expect(await res.json()).toEqual({ error: 'readOnly' })
    expect((await env.CACHE.list()).objects).toHaveLength(0)
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
})
