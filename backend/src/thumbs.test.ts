import { env } from 'cloudflare:test'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { allowRepo, api, breakD1, breakD1Batch, fakeB2, repoPath, reservedBytes, SCHEMA, sha, unsizedBody, useApiHarness } from '../test/helpers'
import { STORAGE_CAP_BYTES } from './storage'
import { decodePack, encodePack, type PackEntry } from './pack'

vi.mock('./redis', async (importOriginal) => {
  const { fakeRedis: fake } = await import('../test/fake-redis')
  return { ...(await importOriginal<typeof import('./redis')>()), redis: () => fake }
})

useApiHarness()
afterEach(() => vi.useRealTimers())

const bytesFor = (n: number) => new Uint8Array([n % 256, (n >> 8) % 256, 7, 7])
const entry = (n: number, suffix = ''): PackEntry => ({ name: `${sha(n)}${suffix}`, bytes: bytesFor(n) })
const putPack = (entries: PackEntry[], headers: Record<string, string> = SCHEMA, repo = 'game') =>
  api(repoPath(repo, '/thumbs/put'), { method: 'POST', body: encodePack(entries), headers })
const getPack = (names: unknown, headers: Record<string, string> = SCHEMA) =>
  api(repoPath('game', '/thumbs/get'), { method: 'POST', body: JSON.stringify({ names }), headers })
const list = async (query = '') => {
  const res = await api(repoPath('game', `/thumbs${query}`), { headers: SCHEMA })
  return { res, body: (await res.json()) as { items: Array<{ name: string; size: number; rev: number }>; next?: string } }
}

describe('thumbnails', () => {
  it('round trip 40 thumbs', async () => {
    allowRepo('game', true)
    const first = Array.from({ length: 20 }, (_, i) => entry(i + 1))
    const second = Array.from({ length: 20 }, (_, i) => entry(i + 21, i % 2 ? '.fx' : ''))
    expect((await putPack(first)).status).toBe(200)
    expect((await putPack(second)).status).toBe(200)

    const all = [...first, ...second]
    const res = await getPack(all.map((e) => e.name))
    expect(res.status).toBe(200)
    expect(res.headers.get('content-type')).toBe('application/octet-stream')
    const decoded = decodePack(new Uint8Array(await res.arrayBuffer()))
    expect(decoded).toHaveLength(40)
    const byName = new Map(decoded.map((e) => [e.name, e.bytes]))
    for (const e of all) expect(byName.get(e.name)).toEqual(e.bytes)
  })

  it('leaves missing names out and returns nothing for none found', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    const res = await getPack([sha(1), sha(2)])
    expect(decodePack(new Uint8Array(await res.arrayBuffer())).map((e) => e.name)).toEqual([sha(1)])
    const none = await getPack([sha(9)])
    expect((await none.arrayBuffer()).byteLength).toBe(0)
  })

  it('rejects 41 names', async () => {
    allowRepo('game')
    const res = await getPack(Array.from({ length: 41 }, (_, i) => sha(i + 1)))
    expect(res.status).toBe(400)
  })

  it('rejects bad name', async () => {
    allowRepo('game', true)
    for (const bad of ['zz', sha(255).toUpperCase(), `${sha(1)}.png`, `../${sha(1)}`, 5]) {
      expect((await getPack([bad])).status).toBe(400)
    }
    expect((await putPack([{ name: 'nope', bytes: new Uint8Array([1]) }])).status).toBe(400)
    expect((await getPack('not-an-array')).status).toBe(400)
  })

  it('rejects a bad pack, more than 20 entries, and more than 4 MB', async () => {
    allowRepo('game', true)
    const truncated = encodePack([entry(1)]).subarray(0, 10)
    expect((await api(repoPath('game', '/thumbs/put'), { method: 'POST', body: truncated, headers: SCHEMA })).status).toBe(400)
    expect((await putPack(Array.from({ length: 21 }, (_, i) => entry(i + 1)))).status).toBe(400)
    expect((await putPack([])).status).toBe(400)
    const big = { name: sha(1), bytes: new Uint8Array(4 * 1024 * 1024) }
    expect((await putPack([big])).status).toBe(413)
  })

  it('needs a schema header', async () => {
    allowRepo('game', true)
    expect((await putPack([entry(1)], {})).status).toBe(400)
    expect((await getPack([sha(1)], {})).status).toBe(400)
    const res = await api(repoPath('game', '/thumbs'))
    expect(res.status).toBe(400)
  })

  it('keeps schemas apart', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    const other = await getPack([sha(1)], { 'x-asset-hub-schema': '2' })
    expect((await other.arrayBuffer()).byteLength).toBe(0)
    expect(fakeB2.keys()).toEqual([`repos/supercent-io/game/s1/thumbs/${sha(1)}`])
  })

  it('cursor handles equal timestamps', async () => {
    allowRepo('game')
    const names = Array.from({ length: 25 }, (_, i) => `${sha(i + 1)}${i % 5 === 0 ? '.fx' : ''}`)
    const insert = env.DB.prepare('INSERT INTO thumbs (repo, schema, name, size, rev) VALUES (?, 1, ?, 4, ?)')
    await env.DB.batch(names.map((n) => insert.bind('supercent-io/game', n, 1000)))
    await env.DB.prepare('INSERT INTO thumbs (repo, schema, name, size, rev) VALUES (?, 2, ?, 4, 1000)').bind('supercent-io/game', sha(99)).run()

    const seen: string[] = []
    let after = ''
    for (let pages = 0; pages < 10; pages++) {
      const { body } = await list(`?limit=10${after}`)
      seen.push(...body.items.map((i) => i.name))
      if (!body.next) break
      after = `&after=${body.next}`
    }
    expect(seen).toHaveLength(25)
    expect(new Set(seen).size).toBe(25)
    expect([...seen].sort()).toEqual([...names].sort())
  })

  it('lists in (rev, name) order and omits next on the last page', async () => {
    allowRepo('game')
    const insert = env.DB.prepare('INSERT INTO thumbs (repo, schema, name, size, rev) VALUES (?, 1, ?, 4, ?)')
    await env.DB.batch([insert.bind('supercent-io/game', sha(2), 5), insert.bind('supercent-io/game', sha(1), 5), insert.bind('supercent-io/game', sha(3), 4)])
    const { body } = await list()
    expect(body.items.map((i) => i.name)).toEqual([sha(3), sha(1), sha(2)])
    expect(body.next).toBeUndefined()
    const exact = await list('?limit=3')
    expect(exact.body.next).toBeUndefined()
    const partial = await list('?limit=2')
    expect(partial.body.next).toBe(`5:${sha(1)}`)
  })

  it('rejects a bad cursor or limit', async () => {
    allowRepo('game')
    expect((await list('?after=garbage')).res.status).toBe(400)
    expect((await list('?limit=0')).res.status).toBe(400)
    expect((await list('?limit=1001')).res.status).toBe(400)
  })

  it('put needs push', async () => {
    allowRepo('game', false)
    const res = await putPack([entry(1)])
    expect(res.status).toBe(403)
    expect(await res.json()).toEqual({ error: 'readOnly' })
    expect(fakeB2.keys()).toEqual([])
    expect((await env.DB.prepare('SELECT COUNT(*) AS n FROM thumbs').first<{ n: number }>())!.n).toBe(0)
  })

  it('replaced thumb listed again', async () => {
    allowRepo('game', true)
    await putPack([entry(1), entry(2)])
    const first = await list()
    expect(first.body.items.map((i) => i.rev)).toEqual([1, 1])
    const cursor = `${first.body.items.at(-1)!.rev}:${first.body.items.at(-1)!.name}`
    expect((await list(`?after=${cursor}`)).body.items).toEqual([])

    await putPack([{ name: sha(1), bytes: new Uint8Array([9, 9, 9, 9, 9]) }])
    const again = await list(`?after=${cursor}`)
    expect(again.body.items).toEqual([{ name: sha(1), size: 5, rev: 2 }])
    const res = await getPack([sha(1)])
    expect(decodePack(new Uint8Array(await res.arrayBuffer()))[0].bytes).toEqual(new Uint8Array([9, 9, 9, 9, 9]))
  })

  it('replacement in same millisecond is listed again', async () => {
    allowRepo('game', true)
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(new Date('2026-11-09T00:00:00Z'))
    await putPack([entry(1)])
    const cursor = `${(await list()).body.items[0].rev}:${sha(1)}`
    // Same clock reading: wall-clock time cannot tell the two commits apart, the revision can.
    await putPack([{ name: sha(1), bytes: new Uint8Array([1, 2, 3, 4, 5, 6]) }])
    const again = await list(`?after=${cursor}`)
    expect(again.body.items).toEqual([{ name: sha(1), size: 6, rev: 2 }])
  })

  it('revisions are kept per repo and schema', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    await putPack([entry(2)], { 'x-asset-hub-schema': '2' })
    await putPack([entry(3)])
    const rows = await env.DB.prepare('SELECT schema, name, rev FROM thumbs ORDER BY schema, name').all<{ schema: number; name: string; rev: number }>()
    expect(rows.results).toEqual([
      { schema: 1, name: sha(1), rev: 1 },
      { schema: 1, name: sha(3), rev: 2 },
      { schema: 2, name: sha(2), rev: 1 }
    ])
  })

  it('get streams entries in order', async () => {
    allowRepo('game', true)
    await putPack([entry(1), entry(2), entry(3)])
    const order = [sha(3), sha(9), sha(1), sha(2)]
    const res = await getPack(order)
    expect(res.body).toBeInstanceOf(ReadableStream)
    const decoded = decodePack(new Uint8Array(await res.arrayBuffer()))
    expect(decoded.map((e) => e.name)).toEqual([sha(3), sha(1), sha(2)])
    expect(decoded[0].bytes).toEqual(bytesFor(3))
  })

  it('get reads B2 only as the client consumes', async () => {
    allowRepo('game', true)
    await putPack(Array.from({ length: 20 }, (_, i) => entry(i + 1)))
    fakeB2.calls = []
    const res = await getPack(Array.from({ length: 20 }, (_, i) => sha(i + 1)))
    const reader = res.body!.getReader()
    await reader.read()
    expect(fakeB2.callsTo('GET').length).toBeLessThanOrEqual(6)
    await reader.cancel()
  })

  it('get runs at most 6 B2 requests at once and keeps request order', async () => {
    allowRepo('game', true)
    const names = Array.from({ length: 40 }, (_, i) => sha(i + 1))
    await putPack(names.slice(0, 20).map((_n, i) => entry(i + 1)))
    await putPack(names.slice(20).map((_n, i) => entry(i + 21)))
    // Later names answer first, so only an ordered writer keeps the request order.
    fakeB2.hold = async (call) => {
      if (call.method === 'GET') await new Promise((r) => setTimeout(r, 40 - Number.parseInt(call.key.slice(-40), 16)))
    }
    const order = [...names].reverse()
    const decoded = decodePack(new Uint8Array(await (await getPack(order)).arrayBuffer()))
    expect(decoded.map((e) => e.name)).toEqual(order)
    expect(fakeB2.maxInflightGets).toBeGreaterThan(1)
    expect(fakeB2.maxInflightGets).toBeLessThanOrEqual(6)
  })

  it('a replaced thumbnail deletes its old version and releases its bytes', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    const key = `repos/supercent-io/game/s1/thumbs/${sha(1)}`
    expect(fakeB2.versionsOf(key)).toHaveLength(1)
    await putPack([{ name: sha(1), bytes: new Uint8Array([9, 9, 9, 9, 9]) }])
    expect(fakeB2.versionsOf(key)).toHaveLength(1)
    expect(await reservedBytes()).toBe(5)
    expect(fakeB2.callsTo('DELETE').every((c) => c.versionId)).toBe(true)
  })

  it('507 at the cap leaves B2 and the index untouched', async () => {
    allowRepo('game', true)
    await env.DB.prepare('UPDATE storage SET reserved_bytes = ?').bind(STORAGE_CAP_BYTES - 3).run()
    const res = await putPack([entry(1)])
    expect(res.status).toBe(507)
    expect(await res.json()).toEqual({ error: 'storageFull' })
    expect(fakeB2.calls).toEqual([])
    expect((await list()).body.items).toEqual([])
  })

  it('a refused upload undoes the versions that landed', async () => {
    allowRepo('game', true)
    fakeB2.failWith = (call) => (call.method === 'PUT' && call.key.endsWith(sha(2)) ? 503 : null)
    const res = await putPack([entry(1), entry(2), entry(3)])
    expect(res.status).toBe(500)
    expect(fakeB2.keys()).toEqual([])
    expect(await reservedBytes()).toBe(0)
    expect((await list()).body.items).toEqual([])
  })

  it('a name sent twice stores one version', async () => {
    allowRepo('game', true)
    await putPack([entry(1), { name: sha(1), bytes: new Uint8Array([5, 5]) }])
    expect(fakeB2.versionsOf(`repos/supercent-io/game/s1/thumbs/${sha(1)}`)).toHaveLength(1)
    expect(await reservedBytes()).toBe(2)
  })

  it('put rejects thumbnail over 1 MB', async () => {
    allowRepo('game', true)
    const res = await putPack([{ name: sha(1), bytes: new Uint8Array(1024 * 1024 + 1) }])
    expect(res.status).toBe(413)
    expect(fakeB2.keys()).toEqual([])
    expect((await putPack([{ name: sha(1), bytes: new Uint8Array(1024 * 1024) }])).status).toBe(200)
  })

  it('rejects oversized body without content-length', async () => {
    allowRepo('game', true)
    const { body, pulled } = unsizedBody(100, 64 * 1024) // 6.4 MB, limit 4 MB
    const res = await api(repoPath('game', '/thumbs/put'), { method: 'POST', body, headers: SCHEMA })
    expect(res.status).toBe(413)
    expect(pulled()).toBeLessThan(100)
  })

  it('concurrent puts get distinct revs', async () => {
    allowRepo('game', true)
    const results = await Promise.all([putPack([entry(1)]), putPack([entry(2)]), putPack([entry(3)])])
    expect(results.map((r) => r.status)).toEqual([200, 200, 200])
    const { body } = await list()
    expect(body.items.map((i) => i.rev).sort()).toEqual([1, 2, 3])
  })

  it('cancel aborts B2 requests that are still in flight', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    let aborted = false
    fakeB2.hold = (_call, signal) =>
      new Promise<void>((_resolve, reject) =>
        signal.addEventListener('abort', () => {
          aborted = true
          reject(new DOMException('aborted', 'AbortError'))
        })
      )
    const res = await getPack([sha(1)])
    const reader = res.body!.getReader()
    const pending = reader.read().catch(() => {}) // starts the lookup, which stays pending
    await vi.waitFor(() => expect(fakeB2.callsTo('GET')).toHaveLength(1))
    await reader.cancel()
    await pending
    await vi.waitFor(() => expect(aborted).toBe(true))
  })

  it('get asks B2 for the version the index names', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    const key = `repos/supercent-io/game/s1/thumbs/${sha(1)}`
    const [indexed] = fakeB2.versionsOf(key)
    // A newer upload that the index does not name must not be served.
    fakeB2.seed(key, new Uint8Array([66, 66]))
    fakeB2.calls = []
    const res = await getPack([sha(1)])
    expect(decodePack(new Uint8Array(await res.arrayBuffer()))[0].bytes).toEqual(bytesFor(1))
    expect(fakeB2.callsTo('GET').map((c) => c.versionId)).toEqual([indexed])
  })

  it('a pack error aborts the sibling downloads', async () => {
    allowRepo('game', true)
    await putPack(Array.from({ length: 6 }, (_, i) => entry(i + 1)))
    let aborted = 0
    fakeB2.failWith = (call) => (call.key.endsWith(sha(1)) ? 503 : null)
    fakeB2.hold = (call, signal) =>
      call.key.endsWith(sha(1))
        ? Promise.resolve()
        : new Promise<void>((_resolve, reject) =>
            signal.addEventListener('abort', () => {
              aborted += 1
              reject(new DOMException('aborted', 'AbortError'))
            })
          )
    const res = await getPack(Array.from({ length: 6 }, (_, i) => sha(i + 1)))
    await expect(res.body!.getReader().read()).rejects.toBeTruthy()
    await vi.waitFor(() => expect(aborted).toBe(5))
  })

  const THUMB_ROWS = /^SELECT name, version_id FROM thumbs/

  it('a batch that threw but landed keeps the live versions and their bytes', async () => {
    allowRepo('game', true)
    breakD1Batch('after')
    expect((await putPack([entry(1), entry(2)])).status).toBe(500)
    expect(fakeB2.keys()).toHaveLength(2)
    expect(await reservedBytes()).toBe(8)
    expect(fakeB2.callsTo('DELETE')).toEqual([])
  })

  it('a batch that threw and did not land deletes the new versions and releases their bytes', async () => {
    allowRepo('game', true)
    breakD1Batch('before')
    expect((await putPack([entry(1), entry(2)])).status).toBe(500)
    expect(fakeB2.keys()).toEqual([])
    expect(await reservedBytes()).toBe(0)
  })

  it('when the batch outcome cannot be read, nothing is deleted and the bytes stay charged', async () => {
    allowRepo('game', true)
    breakD1Batch('after')
    breakD1({ sql: THUMB_ROWS, mode: 'before' })
    expect((await putPack([entry(1), entry(2)])).status).toBe(500)
    expect(fakeB2.keys()).toHaveLength(2)
    expect(fakeB2.callsTo('DELETE')).toEqual([])
    expect(await reservedBytes()).toBe(8)
  })
})
