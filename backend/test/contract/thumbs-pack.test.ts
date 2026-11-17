import { describe, expect, it, vi } from 'vitest'
import { allowRepo, api, b2, db, repoPath, reservedBytes, restoreShardTable, runScheduled, SCHEMA, sha, shardDb, THUMB_SHARDS, unsizedBody, useContract, waitForLog } from './harness'
import { expectErrorReplies, expectNoCors, headerNames, JSON_TYPE, snapshot, type Snapshot } from './raw'
import {
  ageBlobs,
  blobRows,
  bytesFor,
  decodePack,
  encodePack,
  entry,
  entryIn,
  getPack,
  indexedGen,
  list,
  nameIn,
  putPack,
  seedBlob,
  seedBlobs,
  seedThumbs,
  setShardUsage,
  shardOf,
  shardUsage,
  type PackEntry
} from './thumbs-helpers'

useContract()

/** A shard refuses to grow past 300 MiB of payload. */
const SHARD_CAP_BYTES = 300 * 1024 * 1024

/** The most bytes of one thumbnail. */
const THUMB_LIMIT = 128 * 1024

/** Makes the next upload hit the storage cap of every shard. */
async function fillStorage(): Promise<void> {
  for (let shard = 0; shard < THUMB_SHARDS; shard++) await setShardUsage(shard, SHARD_CAP_BYTES)
}

/** Nothing was stored for any thumbnail in any shard. */
async function expectNothingStored(): Promise<void> {
  expect(await blobRows()).toEqual([])
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
    expect((await blobRows()).map((r) => [r.schema, r.name])).toEqual([[1, sha(1)]])
  })

  it('put needs push', async () => {
    allowRepo('game', false)
    const res = await putPack([entry(1)])
    expect(res.status).toBe(403)
    expect(await res.json()).toEqual({ error: 'readOnly' })
    await expectNothingStored()
    expect((await list()).body.items).toEqual([])
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
    await putPack([entry(1)])
    // The Worker's clock cannot be frozen over HTTP, so a real same-millisecond pair cannot be forced here
    // (a frozen-clock check belongs in a unit test over the revision allocator). This puts replacements back
    // to back, which lands several of them in one millisecond on a fast machine. Each one must get a higher
    // revision than the last and show up after the cursor of the one before.
    let cursor = `${(await list()).body.items[0].rev}:${sha(1)}`
    let last = 1
    for (let size = 2; size <= 7; size++) {
      await putPack([{ name: sha(1), bytes: new Uint8Array(size) }])
      const again = await list(`?after=${cursor}`)
      expect(again.body.items).toHaveLength(1)
      expect(again.body.items[0]).toMatchObject({ name: sha(1), size })
      expect(again.body.items[0].rev).toBeGreaterThan(last)
      last = again.body.items[0].rev
      cursor = `${last}:${sha(1)}`
    }
  })

  it('revisions are kept per repo and schema', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    await putPack([entry(2)], { 'x-asset-hub-schema': '2' })
    await putPack([entry(3)])
    const one = await api(repoPath('game', '/thumbs'), { headers: SCHEMA })
    const two = await api(repoPath('game', '/thumbs'), { headers: { 'x-asset-hub-schema': '2' } })
    expect(((await one.json()) as { items: unknown[] }).items).toEqual([
      { name: sha(1), size: 4, rev: 1 },
      { name: sha(3), size: 4, rev: 2 }
    ])
    expect(((await two.json()) as { items: unknown[] }).items).toEqual([{ name: sha(2), size: 4, rev: 1 }])
  })

  it('get streams entries in order', async () => {
    allowRepo('game', true)
    await putPack([entry(1), entry(2), entry(3)])
    const order = [sha(3), sha(9), sha(1), sha(2)]
    const res = await getPack(order)
    expect(res.body).not.toBeNull()
    const decoded = decodePack(new Uint8Array(await res.arrayBuffer()))
    expect(decoded.map((e) => e.name)).toEqual([sha(3), sha(1), sha(2)])
    expect(decoded[0].bytes).toEqual(bytesFor(3))
  })

  it('put rejects a thumbnail over the size limit', async () => {
    allowRepo('game', true)
    const res = await putPack([{ name: sha(1), bytes: new Uint8Array(THUMB_LIMIT + 1) }])
    expect(res.status).toBe(413)
    await expectNothingStored()
    expect((await putPack([{ name: sha(1), bytes: new Uint8Array(THUMB_LIMIT) }])).status).toBe(200)
  })

  it('rejects oversized body without content-length', async () => {
    allowRepo('game', true)
    // 25 MB offered in 64 KB chunks, limit 4 MB: the Worker must stop reading well before the end.
    const { body, pulled } = unsizedBody(400, 64 * 1024, undefined, 1)
    const res = await api(repoPath('game', '/thumbs/put'), { method: 'POST', body, headers: SCHEMA })
    expect(res.status).toBe(413)
    expect(pulled()).toBeLessThan(400)
  })

  it('concurrent puts get distinct revs', async () => {
    allowRepo('game', true)
    const results = await Promise.all([putPack([entry(1)]), putPack([entry(2)]), putPack([entry(3)])])
    expect(results.map((r) => r.status)).toEqual([200, 200, 200])
    const { body } = await list()
    expect(body.items.map((i) => i.rev).sort()).toEqual([1, 2, 3])
  })

})

describe('thumbnails in D1 shards', () => {
  /** One entry in each of the 16 first-digit groups: all 8 shards, two names each. */
  const spread = Array.from({ length: 16 }, (_, digit) => entryIn(digit, digit + 1, digit % 3 === 0 ? '.fx' : ''))

  it('round trip across all shards and keeps request order', async () => {
    allowRepo('game', true)
    expect((await putPack(spread)).status).toBe(200)
    expect(new Set((await blobRows()).map((r) => r.shard)).size).toBe(THUMB_SHARDS)
    for (const row of await blobRows()) expect(row.shard).toBe(shardOf(row.name))

    const order = [...spread].reverse().map((e) => e.name)
    const res = await getPack(order)
    expect(res.status).toBe(200)
    const decoded = decodePack(new Uint8Array(await res.arrayBuffer()))
    expect(decoded.map((e) => e.name)).toEqual(order)
    const byName = new Map(spread.map((e) => [e.name, e.bytes]))
    for (const e of decoded) expect(e.bytes).toEqual(byName.get(e.name))
  })

  it('a thumbnail of the largest size round trips through a shard', async () => {
    allowRepo('game', true)
    const big = [1, 2, 3].map((n) => ({ name: nameIn(n, n), bytes: Uint8Array.from({ length: THUMB_LIMIT }, (_, i) => (i * 7 + n) % 256) }))
    expect((await putPack(big)).status).toBe(200)
    const decoded = decodePack(new Uint8Array(await (await getPack(big.map((e) => e.name))).arrayBuffer()))
    expect(decoded).toHaveLength(3)
    for (const [i, e] of decoded.entries()) expect(Buffer.compare(Buffer.from(e.bytes), Buffer.from(big[i].bytes))).toBe(0)
  })

  it('a full pack of mixed sizes stores every entry once, lists the sizes, and a replace with other sizes serves the new bytes', async () => {
    allowRepo('game', true)
    const sizeOf = (i: number) => (i === 3 ? 62 * 1024 : i === 11 ? THUMB_LIMIT : 3000 + ((i * 937) % 6000))
    const pack = (salt: number): PackEntry[] =>
      Array.from({ length: 20 }, (_, i) => ({ name: nameIn(i % 16, i + 1), bytes: Uint8Array.from({ length: sizeOf(i) - salt * 17 }, (_, k) => (k * 13 + i + salt) % 256) }))
    const first = pack(0)
    expect((await putPack(first)).status).toBe(200)
    expect(await blobRows()).toHaveLength(20)
    const sizes = new Map((await list()).body.items.map((e) => [e.name, e.size]))
    for (const e of first) expect(sizes.get(e.name)).toBe(e.bytes.length)

    const second = pack(1)
    expect((await putPack(second)).status).toBe(200)
    await vi.waitFor(async () => expect(await blobRows()).toHaveLength(20))
    const decoded = decodePack(new Uint8Array(await (await getPack(second.map((e) => e.name))).arrayBuffer()))
    expect(decoded.map((e) => e.name)).toEqual(second.map((e) => e.name))
    for (const [i, e] of decoded.entries()) expect(Buffer.compare(Buffer.from(e.bytes), Buffer.from(second[i].bytes))).toBe(0)
    const revs = new Set((await list()).body.items.map((e) => e.rev))
    expect(revs.size).toBe(1)
  })

  it('no thumbnail route calls B2', async () => {
    allowRepo('game', true)
    await putPack(spread)
    await putPack([{ name: spread[0].name, bytes: new Uint8Array([9, 9]) }])
    await getPack(spread.map((e) => e.name))
    await list()
    expect(b2.calls).toEqual([])
  })

  it('get serves the generation the index names', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    // A newer row that the index does not name must not be served.
    await seedBlob({ repo: 'supercent-io/game', schema: 1, name: sha(1), gen: 'zzzz-newer', bytes: new Uint8Array([66, 66]) })
    const res = await getPack([sha(1)])
    expect(decodePack(new Uint8Array(await res.arrayBuffer()))[0].bytes).toEqual(bytesFor(1))
  })

  it('a row with no index entry is left out', async () => {
    allowRepo('game', true)
    await seedBlob({ repo: 'supercent-io/game', schema: 1, name: sha(7), gen: 'loose', bytes: new Uint8Array([1]) })
    expect((await (await getPack([sha(7)])).arrayBuffer()).byteLength).toBe(0)
  })

  it('a replace deletes the old generation and lowers shard usage', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    const [first] = await blobRows()
    expect(await shardUsage(0)).toBe(4)
    await putPack([{ name: sha(1), bytes: new Uint8Array([9, 9, 9, 9, 9]) }])
    const rows = await blobRows()
    expect(rows).toHaveLength(1)
    expect(rows[0].gen).not.toBe(first.gen)
    expect(rows[0].gen).toBe(await indexedGen(sha(1)))
    expect(await shardUsage(0)).toBe(5)
    expect(await reservedBytes()).toBe(0)
  })

  it('a name sent twice stores one generation', async () => {
    allowRepo('game', true)
    await putPack([entry(1), { name: sha(1), bytes: new Uint8Array([5, 5]) }])
    expect(await blobRows()).toHaveLength(1)
    expect(await shardUsage(0)).toBe(2)
    const decoded = decodePack(new Uint8Array(await (await getPack([sha(1)])).arrayBuffer()))
    expect(decoded[0].bytes).toEqual(new Uint8Array([5, 5]))
  })

  it('507 at the shard cap leaves the shards and the index untouched', async () => {
    allowRepo('game', true)
    await setShardUsage(0, SHARD_CAP_BYTES - 3)
    // Shard 1 takes its reservation first and must give it back when shard 0 refuses.
    const res = await putPack([entryIn(1, 1), entry(1)])
    expect(res.status).toBe(507)
    expect(await res.json()).toEqual({ error: 'storageFull' })
    expect(await blobRows()).toEqual([])
    expect(await shardUsage(0)).toBe(SHARD_CAP_BYTES - 3)
    expect(await shardUsage(1)).toBe(0)
    expect((await list()).body.items).toEqual([])
    expect(b2.calls).toEqual([])
    // The same shard still takes what fits.
    expect((await putPack([{ name: sha(2), bytes: new Uint8Array(3) }])).status).toBe(200)
  })

  it('a shard failure on GET gives 503 and no pack bytes', async () => {
    allowRepo('game', true)
    await putPack(spread)
    const broken = shardOf(spread[3].name)
    await (await shardDb(broken)).prepare('DROP TABLE blobs').run()
    try {
      const res = await getPack(spread.map((e) => e.name))
      expect(res.status).toBe(503)
      expect(await res.text()).toBe('{"error":"storageUnavailable"}')
      expect(res.headers.get('content-type')).toBe(JSON_TYPE)
      // A GET that never touches the broken shard still works.
      const other = spread.find((e) => shardOf(e.name) !== broken)!
      const ok = await getPack([other.name])
      expect(ok.status).toBe(200)
      expect(decodePack(new Uint8Array(await ok.arrayBuffer()))).toHaveLength(1)
      await waitForLog('thumbs.read')
    } finally {
      await restoreShardTable(broken, 'blobs')
    }
  })

  it('a shard failure on PUT undoes the other shards and gives 503', async () => {
    allowRepo('game', true)
    const pack = [0, 1, 2, 3, 4].map((digit) => entryIn(digit, 1))
    await (await shardDb(3)).prepare('DROP TABLE blobs').run()
    try {
      const res = await putPack(pack)
      expect(res.status).toBe(503)
      expect(await res.json()).toEqual({ error: 'storageUnavailable' })
      // The shards that took their rows give them back and release their bytes. The broken shard cannot be read.
      for (const shard of [0, 1, 2, 4]) {
        const rows = await (await shardDb(shard)).prepare('SELECT COUNT(*) AS n FROM blobs').first<{ n: number }>()
        expect(rows!.n).toBe(0)
        expect(await shardUsage(shard)).toBe(0)
      }
      expect((await list()).body.items).toEqual([])
      expect(b2.calls).toEqual([])
    } finally {
      await restoreShardTable(3, 'blobs')
    }
  })

  it('concurrent writers of one name leave exactly one generation', async () => {
    allowRepo('game', true)
    for (let round = 1; round <= 5; round++) {
      const name = nameIn(round, round)
      const results = await Promise.all(
        [3, 5, 7, 9].map((size) => putPack([{ name, bytes: new Uint8Array(size).fill(size) }]))
      )
      expect(results.map((r) => r.status)).toEqual([200, 200, 200, 200])
      const rows = (await blobRows()).filter((r) => r.name === name)
      expect(rows).toHaveLength(1)
      expect(rows[0].gen).toBe(await indexedGen(name))
      expect(await shardUsage(shardOf(name))).toBe(rows[0].size)
      const decoded = decodePack(new Uint8Array(await (await getPack([name])).arrayBuffer()))
      expect(decoded[0].bytes).toHaveLength(rows[0].size)
    }
  })

  it('shards of one name are kept per repo and schema', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    await putPack([{ name: sha(1), bytes: new Uint8Array([8, 8]) }], { 'x-asset-hub-schema': '2' })
    const rows = await blobRows()
    expect(rows.map((r) => [r.schema, r.size]).sort()).toEqual([[1, 4], [2, 2]])
    expect(await shardUsage(0)).toBe(6)
  })

  it('rejects an entry of 128 KiB plus one byte and stores nothing of its pack', async () => {
    allowRepo('game', true)
    const pack = [entry(1), { name: sha(2), bytes: new Uint8Array(THUMB_LIMIT + 1) }]
    const res = await putPack(pack)
    expect(res.status).toBe(413)
    expect(await res.json()).toEqual({ error: 'tooLarge' })
    expect(await blobRows()).toEqual([])
    expect(await shardUsage(0)).toBe(0)
    expect((await list()).body.items).toEqual([])
  })

  it('stores the creation time of each row', async () => {
    allowRepo('game', true)
    const before = Date.now()
    await putPack([entry(1)])
    const row = await (await shardDb(0)).prepare('SELECT created FROM blobs').first<{ created: number }>()
    expect(row!.created).toBeGreaterThanOrEqual(before - 1000)
    expect(row!.created).toBeLessThanOrEqual(Date.now() + 1000)
  })

  describe('rows of the TypeScript Worker', () => {
    const b2Version = '4_z27c88f1d182b150646ff0b16_f2142e4dc1b0e0c88_d20250101_m000000_c004_v0402005_t0001_u01735689600000'

    it('are not served even when a shard row has the same id', async () => {
      allowRepo('game', true)
      await seedThumbs([{ repo: 'supercent-io/game', schema: 1, name: sha(1), rev: 1, versionId: b2Version }])
      await seedBlob({ repo: 'supercent-io/game', schema: 1, name: sha(1), gen: b2Version, bytes: new Uint8Array([1, 2, 3]) })
      const res = await getPack([sha(1)])
      expect(res.status).toBe(200)
      expect((await res.arrayBuffer()).byteLength).toBe(0)
      expect((await list()).body.items).toEqual([])
    })

    it('are replaced by a PUT of the same name, which lists again and serves the new bytes', async () => {
      allowRepo('game', true)
      await seedThumbs([{ repo: 'supercent-io/game', schema: 1, name: sha(1), rev: 5, versionId: b2Version }])
      await db().then((d) => d.prepare("INSERT INTO thumb_rev (repo, schema, rev) VALUES ('supercent-io/game', 1, 5)").run())
      expect((await putPack([entry(1)])).status).toBe(200)
      const { body } = await list()
      expect(body.items).toEqual([{ name: sha(1), size: 4, rev: 6 }])
      expect(await indexedGen(sha(1))).toBe((await blobRows())[0].gen)
      expect(decodePack(new Uint8Array(await (await getPack([sha(1)])).arrayBuffer()))[0].bytes).toEqual(bytesFor(1))
      expect(await shardUsage(0)).toBe(4)
    })
  })
})

describe('sweep of the shards', () => {
  const HOUR = 60 * 60 * 1000
  const repo = 'supercent-io/game'
  /** A cron trigger whose time makes the run sweep `shard`. */
  const triggerFor = (shard: number) => new Date(shard * 3 * HOUR + 17 * 60_000)
  const bytes = (n: number) => new Uint8Array(n).fill(n)

  it('removes an old orphan, keeps a live row and a young orphan, and counts the usage again', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    await ageBlobs(sha(1), 2 * HOUR)
    await seedBlobs([
      { repo, schema: 1, name: sha(2), gen: 'orphan', bytes: bytes(5), created: Date.now() - 2 * HOUR },
      { repo, schema: 1, name: sha(3), gen: 'young', bytes: bytes(7), created: Date.now() - HOUR / 6 }
    ])
    await setShardUsage(0, 999)
    await runScheduled(triggerFor(0))
    const rows = await blobRows()
    expect(rows.map((r) => [r.name, r.gen])).toEqual([
      [sha(1), await indexedGen(sha(1))],
      [sha(3), 'young']
    ])
    expect(await shardUsage(0)).toBe(4 + 7)
    // The live row still serves.
    expect(decodePack(new Uint8Array(await (await getPack([sha(1)])).arrayBuffer()))[0].bytes).toEqual(bytesFor(1))
  })

  it('sweeps one shard for each run and leaves the others alone', async () => {
    allowRepo('game', true)
    await seedBlobs([
      { repo, schema: 1, name: nameIn(0, 1), gen: 'orphan', bytes: bytes(3), created: Date.now() - 2 * HOUR },
      { repo, schema: 1, name: nameIn(1, 1), gen: 'orphan', bytes: bytes(3), created: Date.now() - 2 * HOUR }
    ])
    await runScheduled(triggerFor(1))
    expect((await blobRows()).map((r) => r.shard)).toEqual([0])
    await runScheduled(triggerFor(8))
    expect(await blobRows()).toEqual([])
  })

  it('does not take a row of another repo or schema for the live one', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    const live = (await blobRows())[0]
    // The same name and generation under another schema is not named by the index.
    await seedBlob({ repo, schema: 2, name: live.name, gen: live.gen, bytes: bytes(2), created: Date.now() - 2 * HOUR })
    await ageBlobs(sha(1), 2 * HOUR)
    await runScheduled(triggerFor(0))
    expect((await blobRows()).map((r) => [r.schema, r.gen])).toEqual([[1, live.gen]])
  })

  it('goes through a long table over several runs', async () => {
    allowRepo('game', true)
    const rows = Array.from({ length: 100 }, (_, i) => ({ repo, schema: 1, name: nameIn(0, i + 1), gen: `orphan-${i}`, bytes: bytes(2), created: Date.now() - 2 * HOUR }))
    await seedBlobs(rows)
    await runScheduled(triggerFor(0))
    const left = (await blobRows()).length
    expect(left).toBe(4)
    await runScheduled(triggerFor(0))
    expect(await blobRows()).toEqual([])
    expect(await shardUsage(0)).toBe(0)
  })

  it('leaves a shard it cannot read alone and still rebuilds the popularity', async () => {
    allowRepo('game', true)
    await seedBlobs([{ repo, schema: 1, name: nameIn(2, 1), gen: 'orphan', bytes: bytes(3), created: Date.now() - 2 * HOUR }])
    await (await shardDb(2)).prepare('DROP TABLE usage').run()
    try {
      await runScheduled(triggerFor(2))
      await waitForLog('thumbs.sweep')
    } finally {
      await restoreShardTable(2, 'usage')
      await (await shardDb(2)).prepare('INSERT INTO usage (id, bytes) VALUES (1, 0)').run()
    }
    expect((await blobRows()).map((r) => r.gen)).toEqual(['orphan'])
  })
})

describe('thumbnail put and get raw bytes and headers', () => {
  it('a put reply is exact', async () => {
    allowRepo('game', true)
    const res = await putPack([entry(1), entry(2)])
    expect(await res.text()).toBe('{"stored":2}')
    expect(res.headers.get('content-type')).toBe(JSON_TYPE)
    expect(headerNames(res)).toEqual(['content-type'])
    expectNoCors(res)
  })

  it('a get reply carries the pack with exactly the octet-stream and no-store headers', async () => {
    allowRepo('game', true)
    await putPack([entry(1)])
    const res = await getPack([sha(1)])
    expect(res.headers.get('content-type')).toBe('application/octet-stream')
    expect(res.headers.get('cache-control')).toBe('private, no-store')
    expect(headerNames(res)).toEqual(['cache-control', 'content-type'])
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(encodePack([entry(1)]))
    expectNoCors(res)
  })

  it('error replies are exact JSON', async () => {
    allowRepo('ro', false)
    allowRepo('game', true)
    const cases: Array<[Snapshot, number, string]> = [
      [await snapshot(await putPack([entry(1)], SCHEMA, 'ro')), 403, '{"error":"readOnly"}'],
      [await snapshot(await getPack(['zz'])), 400, '{"error":"badRequest"}'],
      [await snapshot(await getPack([sha(1)], {})), 400, '{"error":"badSchema"}'],
      [await snapshot(await api(repoPath('game', '/thumbs/put'), { method: 'POST', body: encodePack([entry(1)]).subarray(0, 10), headers: SCHEMA })), 400, '{"error":"badPack"}'],
      [await snapshot(await putPack([{ name: sha(1), bytes: new Uint8Array(THUMB_LIMIT + 1) }])), 413, '{"error":"tooLarge"}']
    ]
    await fillStorage()
    cases.push([await snapshot(await putPack([entry(1)])), 507, '{"error":"storageFull"}'])
    expectErrorReplies(cases)
  })
})
