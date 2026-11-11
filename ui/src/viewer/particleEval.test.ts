import { describe, expect, it } from 'vitest'
import { evalGradient, evalMinMax, evalMinMaxColor, streamTexcoords } from './particleEval'
import type { MinMax, MinMaxColor } from '@shared/types'

const k = (t: number, v: number) => ({ t, v: [v], inS: [0], outS: [0] })

describe('particle values', () => {
  it('evaluates MinMaxCurves like Unity: constant, curve x scalar, random between constants', () => {
    const curve: MinMax = { mode: 1, scalar: 2, min: 1, curve: [k(0, 0), k(1, 1)] }
    expect(evalMinMax(curve, 0, 0)).toBe(0)
    expect(evalMinMax(curve, 0.5, 0)).toBeCloseTo(1)
    expect(evalMinMax(curve, 1, 0)).toBe(2)
    expect(evalMinMax({ mode: 3, scalar: 4, min: 2 }, 0, 0.5)).toBe(3)
    expect(evalMinMax({ mode: 0, scalar: 5, min: 0 }, 0, 0)).toBe(5)
  })

  it('evaluates gradients with colour and alpha keys apart', () => {
    const gradient = {
      colors: [
        { t: 0, r: 1, g: 0, b: 0 },
        { t: 1, r: 0, g: 0, b: 1 }
      ],
      alphas: [
        { t: 0, a: 1 },
        { t: 1, a: 0 }
      ]
    }
    const c: MinMaxColor = { mode: 1, min: [1, 1, 1, 1], max: [1, 1, 1, 1], gradient }
    expect(evalMinMaxColor(c, 0, 0)).toEqual([1, 0, 0, 1])
    expect(evalGradient(gradient, 0.5).map((v) => +v.toFixed(2))).toEqual([0.5, 0, 0.5, 0.5])
    expect(evalMinMaxColor({ mode: 2, max: [1, 1, 1, 1], min: [0, 0, 0, 0] }, 0, 0.25)).toEqual([0.25, 0.25, 0.25, 0.25])
  })

  it('packs streams into TEXCOORDs one after another', () => {
    // UV, UV2 fill TEXCOORD0; Custom1.xyzw is TEXCOORD1.
    expect(streamTexcoords([0, 1, 3, 4, 5, 34]).map((s) => `${s.stream}.${s.component}`)).toEqual(['4.0', '4.1', '5.0', '5.1', '34.0', '34.1', '34.2', '34.3'])
    // UV then Center (3): Center runs on from TEXCOORD0.zw into TEXCOORD1.x.
    expect(streamTexcoords([0, 3, 4, 10]).map((s) => s.stream)).toEqual([4, 4, 10, 10, 10])
  })
})
