import { describe, expect, it } from 'vitest'
import { reviveTangents } from './tangents'

describe('reviveTangents', () => {
  it('turns the null of a step tangent back into Infinity, wherever the curve sits', () => {
    const clip = { transformCurves: [{ keys: [{ t: 0, v: [0, 1], inS: [0, null], outS: [null, null] }] }], other: { inS: 'x', list: [{ outS: [null] }] } }
    reviveTangents(clip)
    expect(clip.transformCurves[0].keys[0].inS).toEqual([0, Infinity])
    expect(clip.transformCurves[0].keys[0].outS).toEqual([Infinity, Infinity])
    expect(clip.other.list[0].outS).toEqual([Infinity])
    expect(clip.other.inS).toBe('x')
  })

  it('leaves a null that is not a tangent alone', () => {
    const v = { a: null, keys: [{ v: [null] }] }
    expect(reviveTangents(v)).toEqual({ a: null, keys: [{ v: [null] }] })
  })
})
