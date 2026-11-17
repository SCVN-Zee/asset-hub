import { describe, expect, it } from 'vitest'
import { mergeKnown } from './catalogScan'

describe('mergeKnown', () => {
  it('returns the same object for an empty list', () => {
    const prev = { effects: new Set(['a']) }
    expect(mergeKnown(prev, 'effects', [])).toBe(prev)
  })

  it('returns the same object when every SHA is known', () => {
    const prev = { effects: new Set(['a', 'b']) }
    expect(mergeKnown(prev, 'effects', ['b', 'a'])).toBe(prev)
  })

  it('returns a new object holding the union when one SHA is new', () => {
    const prev = { effects: new Set(['a']) }
    const next = mergeKnown(prev, 'effects', ['a', 'c'])
    expect(next).not.toBe(prev)
    expect([...(next.effects ?? [])].sort()).toEqual(['a', 'c'])
    expect([...(prev.effects ?? [])]).toEqual(['a'])
  })
})
