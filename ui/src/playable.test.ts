import { describe, expect, it } from 'vitest'
import { budgetFor, historyPaths, historyScore, rankForPlayable, summaryIssues, TIGHT_BUDGET } from './playable'
import type { StatsSummary } from '@shared/types'

const stat = (o: Partial<StatsSummary>): StatsSummary => ({
  triangles: 1000,
  vertices: 1500,
  bones: 0,
  clips: 0,
  materials: 1,
  fileBytes: 100_000,
  textureBytes: 200_000,
  maxTextureSize: 512,
  source: 'thumb',
  ...o
})

describe('playable ranking', () => {
  const items = [
    { id: 'heavy', s: stat({ triangles: 40_000, fileBytes: 6_000_000 }), used: 9 },
    { id: 'unmeasured', s: undefined, used: 1 },
    { id: 'light', s: stat({ triangles: 800, fileBytes: 50_000 }), used: 2 },
    { id: 'mid', s: stat({ triangles: 3000 }), used: 5 }
  ]
  const order = (mode: Parameters<typeof rankForPlayable>[1]) => rankForPlayable(items, mode, (i) => i.s, (i) => i.used).map((i) => i.id)

  it('puts models within budget first, smallest first, unmeasured last', () => {
    expect(summaryIssues(items[0].s!).map((i) => i.key)).toEqual(['triangles', 'fileBytes'])
    expect(order('playable')).toEqual(['light', 'mid', 'heavy', 'unmeasured'])
    expect(order('triangles')).toEqual(['light', 'mid', 'heavy', 'unmeasured'])
    expect(order('smallest')).toEqual(['light', 'mid', 'heavy', 'unmeasured'])
  })

  it('orders by use without measurements, and keeps search order for relevance', () => {
    expect(order('used')).toEqual(['heavy', 'mid', 'light', 'unmeasured'])
    expect(order('relevance')).toEqual(['heavy', 'unmeasured', 'light', 'mid'])
  })

  it('never ranks a looser search match above a closer one', () => {
    const match = (i: { id: string }) => (i.id === 'heavy' ? 100 : 30)
    expect(rankForPlayable(items, 'playable', (i) => i.s, (i) => i.used, match).map((i) => i.id)).toEqual(['heavy', 'light', 'mid', 'unmeasured'])
  })
})

describe('popular order', () => {
  type Row = { id: string; team: number; used: number; personal: number }
  const rows: Row[] = [
    { id: 'plain', team: 0, used: 0, personal: 0 },
    { id: 'branches', team: 0, used: 3, personal: 0 },
    { id: 'favorite', team: 0, used: 0, personal: 2 },
    { id: 'team', team: 5, used: 0, personal: 0 }
  ]
  const order = (list: Row[], match?: (r: Row) => number, team = true) =>
    rankForPlayable(list, 'popular', () => undefined, (r) => r.used, match, undefined, (r) => ({ team: team ? r.team : 0, personal: r.personal })).map((r) => r.id)

  it('team score first', () => {
    expect(order(rows)[0]).toBe('team')
  })

  it('branch count when no team data', () => {
    expect(order(rows, undefined, false)).toEqual(['branches', 'favorite', 'plain', 'team'])
  })

  it('personal history breaks ties', () => {
    const tied = [rows[0], { id: 'viewed', team: 0, used: 0, personal: 1 }, rows[2]]
    expect(order(tied)).toEqual(['favorite', 'viewed', 'plain'])
  })

  it('search keeps relevance first', () => {
    const match = (r: Row) => (r.id === 'plain' ? 100 : 10)
    expect(order(rows, match)).toEqual(['plain', 'team', 'branches', 'favorite'])
  })
})

describe('budget presets', () => {
  it('judges the same asset by the chosen budget', () => {
    const s = stat({ triangles: 10_000, vertices: 0, fileBytes: 2 * 1024 * 1024, textureBytes: 0, maxTextureSize: 800, source: 'viewer' })
    expect(summaryIssues(s, budgetFor('standard'))).toEqual([])
    expect(summaryIssues(s, budgetFor('tight')).map((i) => i.key)).toEqual(['triangles', 'textureSize', 'fileBytes'])
    expect(budgetFor('custom', { triangles: 5000 })).toMatchObject({ triangles: 5000, textureSize: 1024 })
    expect(budgetFor(undefined)).not.toBe(TIGHT_BUDGET)
  })
})

describe('historyScore', () => {
  const scores = new Map([
    ['acme/a\nAssets/Crate.fbx', 3],
    ['acme/b\nAssets/Tree.fbx', 1]
  ])
  const paths = historyPaths(scores)

  it('finds the score by repo and path', () => {
    expect(historyScore(scores, paths, 'acme/a', 'Assets/Crate.fbx')).toBe(3)
    expect(historyScore(scores, paths, 'acme/b', 'Assets/Tree.fbx')).toBe(1)
  })

  it('is 0 for a path outside the history, or the same path in another repo', () => {
    expect(historyScore(scores, paths, 'acme/a', 'Assets/Other.fbx')).toBe(0)
    expect(historyScore(scores, paths, 'acme/b', 'Assets/Crate.fbx')).toBe(0)
  })
})
