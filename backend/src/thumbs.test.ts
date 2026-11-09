import { env } from 'cloudflare:test'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { allowRepo, api, repoPath, SCHEMA, sha, unsizedBody, useApiHarness } from '../test/helpers'
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
    const keys = (await env.CACHE.list()).objects.map((o) => o.key)
    expect(keys).toEqual([`repos/supercent-io/game/s1/thumbs/${sha(1)}`])
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
    expect((await env.CACHE.list()).objects).toHaveLength(0)
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

  it('get reads R2 only as the client consumes', async () => {
    allowRepo('game', true)
    await putPack(Array.from({ length: 20 }, (_, i) => entry(i + 1)))
    const get = vi.spyOn(env.CACHE, 'get')
    const res = await getPack(Array.from({ length: 20 }, (_, i) => sha(i + 1)))
    const reader = res.body!.getReader()
    await reader.read()
    expect(get.mock.calls.length).toBeLessThan(5)
    await reader.cancel()
    get.mockRestore()
  })

  it('put rejects thumbnail over 1 MB', async () => {
    allowRepo('game', true)
    const res = await putPack([{ name: sha(1), bytes: new Uint8Array(1024 * 1024 + 1) }])
    expect(res.status).toBe(413)
    expect((await env.CACHE.list()).objects).toHaveLength(0)
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

  it('cancel during pending lookup cancels the body', async () => {
    allowRepo('game', true)
    let bodyCancelled = false
    let release!: (object: unknown) => void
    const get = vi.spyOn(env.CACHE, 'get').mockImplementation(() => new Promise((resolve) => (release = resolve)) as never)
    try {
      const res = await getPack([sha(1)])
      const reader = res.body!.getReader()
      const pending = reader.read() // starts the lookup, which stays pending
      await vi.waitFor(() => expect(get).toHaveBeenCalledTimes(1))
      await reader.cancel()
      await pending
      release({
        size: 3,
        body: new ReadableStream({
          pull: (c) => c.enqueue(new Uint8Array(3)),
          cancel: () => {
            bodyCancelled = true
          }
        })
      })
      await vi.waitFor(() => expect(bodyCancelled).toBe(true))
    } finally {
      get.mockRestore()
    }
  })
})
