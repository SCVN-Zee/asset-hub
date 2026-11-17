import { describe, expect, it } from 'vitest'
import { allowRepo, api, applyMainMigration, build, db, repoPath, SCHEMA, sha, useContract } from './harness'
import { expectNoCors, JSON_TYPE } from './raw'
import { list, seedThumbs, uuidFor } from './thumbs-helpers'

useContract()

/** The Rust Worker names its thumbnail storage epoch in every list reply. The TypeScript Worker names none. */
const EPOCH = build === 'rust' ? ',"epoch":2' : ''

describe('thumbnail index', () => {
  it('cursor handles equal timestamps', async () => {
    allowRepo('game')
    const names = Array.from({ length: 25 }, (_, i) => `${sha(i + 1)}${i % 5 === 0 ? '.fx' : ''}`)
    await seedThumbs([
      ...names.map((name) => ({ repo: 'supercent-io/game', schema: 1, name, rev: 1000 })),
      { repo: 'supercent-io/game', schema: 2, name: sha(99), rev: 1000 }
    ])

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
    await seedThumbs([
      { repo: 'supercent-io/game', schema: 1, name: sha(2), rev: 5 },
      { repo: 'supercent-io/game', schema: 1, name: sha(1), rev: 5 },
      { repo: 'supercent-io/game', schema: 1, name: sha(3), rev: 4 }
    ])
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
})

describe('thumbnail index raw bytes and headers', () => {
  it('a page with next and a page without next are exact', async () => {
    allowRepo('game')
    await seedThumbs([
      { repo: 'supercent-io/game', schema: 1, name: sha(1), rev: 4 },
      { repo: 'supercent-io/game', schema: 1, name: sha(2), rev: 5 }
    ])
    const first = await api(repoPath('game', '/thumbs?limit=1'), { headers: SCHEMA })
    expect(await first.text()).toBe(`{"items":[{"name":"${sha(1)}","size":4,"rev":4}],"next":"4:${sha(1)}"${EPOCH}}`)
    expect(first.headers.get('content-type')).toBe(JSON_TYPE)
    expectNoCors(first)

    const last = await api(repoPath('game', `/thumbs?limit=1&after=4:${sha(1)}`), { headers: SCHEMA })
    expect(await last.text()).toBe(`{"items":[{"name":"${sha(2)}","size":4,"rev":5}]${EPOCH}}`)
    expect(last.headers.get('content-type')).toBe(JSON_TYPE)
  })

  it('an empty index and the error replies are exact', async () => {
    allowRepo('game')
    expect(await (await api(repoPath('game', '/thumbs'), { headers: SCHEMA })).text()).toBe(`{"items":[]${EPOCH}}`)
    const noSchema = await api(repoPath('game', '/thumbs'))
    expect(noSchema.status).toBe(400)
    expect(await noSchema.text()).toBe('{"error":"badSchema"}')
    expect(noSchema.headers.get('content-type')).toBe(JSON_TYPE)
    const badCursor = await api(repoPath('game', '/thumbs?after=garbage'), { headers: SCHEMA })
    expect(await badCursor.text()).toBe('{"error":"badRequest"}')
    expectNoCors(badCursor)
  })
})

describe.runIf(build === 'rust')('thumbnail index of the Rust Worker', () => {
  const legacy = ['', '4_z27c88f1d182b150646ff0b16_f2142e4dc1b0e0c88_d20250101_m000000_c004_v0402005_t0001_u01735689600000']

  it('says which storage epoch it serves', async () => {
    allowRepo('game')
    const { res, body } = await list()
    expect(res.status).toBe(200)
    expect(body).toEqual({ items: [], epoch: 2 })
  })

  it('does not list a row that names no generation of its own', async () => {
    allowRepo('game')
    await seedThumbs([
      { repo: 'supercent-io/game', schema: 1, name: sha(1), rev: 1, versionId: legacy[0] },
      { repo: 'supercent-io/game', schema: 1, name: sha(2), rev: 2, versionId: legacy[1] },
      { repo: 'supercent-io/game', schema: 1, name: sha(3), rev: 3, versionId: uuidFor(3) },
      { repo: 'supercent-io/game', schema: 1, name: sha(4), rev: 4, versionId: uuidFor(10).toUpperCase() }
    ])
    const { body } = await list()
    expect(body.items.map((i) => i.name)).toEqual([sha(3)])
    // A page is cut by the rows it lists, so a legacy row never takes a place of the limit.
    const first = await list('?limit=1')
    expect(first.body.items.map((i) => i.name)).toEqual([sha(3)])
    expect(first.body.next).toBeUndefined()
  })
})

describe.runIf(build === 'rust')('migration 0003', () => {
  const rows = (names: number[]) => names.map((n) => ({ repo: 'supercent-io/game', schema: 1, name: sha(n), rev: n }))

  it('deletes only the rows of the B2 era, keeps thumb_rev and can run again', async () => {
    await seedThumbs([
      { ...rows([1])[0], versionId: '' },
      { ...rows([2])[0], versionId: '4_zb2version' },
      { ...rows([3])[0], versionId: uuidFor(3) },
      { ...rows([4])[0], versionId: uuidFor(10).toUpperCase() }
    ])
    const database = await db()
    await database.prepare("INSERT INTO thumb_rev (repo, schema, rev) VALUES ('supercent-io/game', 1, 7)").run()
    await applyMainMigration('0003_thumbs_d1.sql')
    const left = await database.prepare('SELECT name FROM thumbs ORDER BY name').all<{ name: string }>()
    expect(left.results.map((r) => r.name)).toEqual([sha(3)])
    expect((await database.prepare('SELECT rev FROM thumb_rev').first<{ rev: number }>())!.rev).toBe(7)
    await applyMainMigration('0003_thumbs_d1.sql')
    expect((await database.prepare('SELECT COUNT(*) AS n FROM thumbs').first<{ n: number }>())!.n).toBe(1)
  })
})
