import { describe, expect, it } from 'vitest'
import { budgetFor, rankForPlayable, summaryIssues, TIGHT_BUDGET } from './playable'
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

describe('budget presets', () => {
  it('judges the same asset by the chosen budget', () => {
    const s = stat({ triangles: 10_000, vertices: 0, fileBytes: 2 * 1024 * 1024, textureBytes: 0, maxTextureSize: 800, source: 'viewer' })
    expect(summaryIssues(s, budgetFor('standard'))).toEqual([])
    expect(summaryIssues(s, budgetFor('tight')).map((i) => i.key)).toEqual(['triangles', 'textureSize', 'fileBytes'])
    expect(budgetFor('custom', { triangles: 5000 })).toMatchObject({ triangles: 5000, textureSize: 1024 })
    expect(budgetFor(undefined)).not.toBe(TIGHT_BUDGET)
  })
})
