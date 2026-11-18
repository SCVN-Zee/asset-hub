import { describe, expect, it } from 'vitest'
import type { StatsSummary } from '@shared/types'
import { mergeStatFact, NO_STAT_FACTS } from './statFacts'

const stat = (o: Partial<StatsSummary> = {}): StatsSummary => ({ triangles: 1, vertices: 1, bones: 0, clips: 0, materials: 1, fileBytes: 1, textureBytes: 1, maxTextureSize: 1, source: 'thumb', ...o })

describe('mergeStatFact', () => {
  it('keeps the same facts when the entry changes nothing they hold', () => {
    const names = new Map<string, string[]>()
    expect(mergeStatFact(NO_STAT_FACTS, names, 'a', undefined, stat())).toBe(NO_STAT_FACTS)
    expect(mergeStatFact(NO_STAT_FACTS, names, 'a', stat(), stat({ triangles: 99 }))).toBe(NO_STAT_FACTS)
  })

  it('changes only the key of the part a new entry touches', () => {
    const names = new Map<string, string[]>()
    const next = mergeStatFact(NO_STAT_FACTS, names, 'a', undefined, stat({ clips: 2, clipNames: ['run', 'idle'] }))
    expect(next).toEqual({ ...NO_STAT_FACTS, clipKey: 1, withClips: 1 })
    expect([...names]).toEqual([['a', ['run', 'idle']]])
    const fx = mergeStatFact(next, names, 'b', undefined, stat({ particles: 5, emitters: 2 }))
    expect(fx).toEqual({ ...next, withParticles: 1 })
  })

  it('moves the clip key when the names change but not the set of models with clips', () => {
    const names = new Map<string, string[]>()
    const first = mergeStatFact(NO_STAT_FACTS, names, 'a', undefined, stat({ clips: 1, clipNames: ['run'] }))
    const renamed = mergeStatFact(first, names, 'a', stat({ clips: 1, clipNames: ['run'] }), stat({ clips: 1, clipNames: ['walk'] }))
    expect(renamed.clipKey).toBe(first.clipKey + 1)
    expect(renamed.withClips).toBe(first.withClips)
    expect(names.get('a')).toEqual(['walk'])
  })

  it('counts the models that are not rigged, up and down', () => {
    const names = new Map<string, string[]>()
    const one = mergeStatFact(NO_STAT_FACTS, names, 'a', undefined, stat({ rigged: false }))
    expect(one.unlisted).toBe(1)
    const two = mergeStatFact(one, names, 'b', undefined, stat({ rigged: false }))
    expect(two.unlisted).toBe(2)
    const back = mergeStatFact(two, names, 'a', stat({ rigged: false }), stat({ rigged: true }))
    expect(back.unlisted).toBe(1)
    expect(back.unlistedKey).toBe(3)
  })
})
