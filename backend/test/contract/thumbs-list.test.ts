import { describe, expect, it } from 'vitest'
import { allowRepo, api, repoPath, SCHEMA, sha, useContract } from './harness'
import { expectNoCors, JSON_TYPE } from './raw'
import { list, seedThumbs } from './thumbs-helpers'

useContract()

/** The Worker names its thumbnail storage epoch in every list reply. */
const EPOCH = ',"epoch":3'

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

describe('thumbnail index', () => {
  it('says which storage epoch it serves', async () => {
    allowRepo('game')
    const { res, body } = await list()
    expect(res.status).toBe(200)
    expect(body).toEqual({ items: [], epoch: 3 })
  })
})
