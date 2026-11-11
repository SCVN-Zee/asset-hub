import { describe, expect, it } from 'vitest'
import { sampleCurve } from './curves'
import type { CurveKey } from '@shared/types'

const flat = (n: number, v = 0): number[] => Array(n).fill(v)
const key = (t: number, v: number[], inS = flat(v.length), outS = flat(v.length)): CurveKey => ({ t, v, inS, outS })

describe('sampleCurve', () => {
  const rot = [key(0, [0, 0, 0, 1]), key(1, [0, 1, 0, 0])]

  it('holds outside the keys and eases between them with Hermite tangents', () => {
    expect(sampleCurve(rot, -1)).toEqual([0, 0, 0, 1])
    expect(sampleCurve(rot, 2)).toEqual([0, 1, 0, 0])
    // Flat tangents: halfway in time is halfway in value.
    expect(sampleCurve(rot, 0.5)[1]).toBeCloseTo(0.5)
    expect(sampleCurve(rot, 0.25)[1]).toBeCloseTo(0.15625)
  })

  it('steps on infinite tangents, which arrive as Infinity', () => {
    const pos = [key(0, [0, 0, 0], flat(3), flat(3, Infinity)), key(1, [0, 5, 0], flat(3, Infinity), flat(3))]
    expect(sampleCurve(pos, 0.25)[1]).toBe(0)
  })

  it('answers nothing for no keys', () => {
    expect(sampleCurve([], 0.5)).toEqual([])
  })
})
