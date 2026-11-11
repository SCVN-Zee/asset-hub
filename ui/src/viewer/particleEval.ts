// Per-frame evaluation of a particle system's values (Unity's MinMaxCurve and MinMaxGradient). Parsing is in Rust;
// these run for every particle on every frame, so they stay here.

import { sampleCurve } from './curves'
import type { Gradient, MinMax, MinMaxColor } from '@shared/types'

/** Components of each ParticleSystemVertexStream (index = Unity's enum value); Position, Normal, Tangent and Color have their own semantics. */
const STREAM_SIZES = [0, 0, 0, 0, 2, 2, 2, 2, 1, 1, 3, 1, 1, 2, 3, 1, 3, 1, 3, 3, 1, 1, 1, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 1, 2, 3]
/** Unity's streams when the renderer lists none: Position, Normal, Color, UV. */
export const DEFAULT_STREAMS = [0, 1, 3, 4]

/**
 * What lands in each TEXCOORD component, in order (TEXCOORD0.xyzw, TEXCOORD1.xyzw…):
 * Unity packs the streams one after another, a stream running on into the next
 * TEXCOORD when it does not fit (`TEXCOORD0.zw|x`).
 */
export function streamTexcoords(streams: readonly number[]): Array<{ stream: number; component: number }> {
  return streams.flatMap((stream) => Array.from({ length: STREAM_SIZES[stream] ?? 0 }, (_, component) => ({ stream, component })))
}

const lerp = (a: number, b: number, t: number) => a + (b - a) * t

/** A MinMaxCurve at normalised time `t` (0–1), for a particle's own random `r` (0–1). */
export function evalMinMax(c: MinMax, t: number, r: number): number {
  switch (c.mode) {
    case 1:
      return c.scalar * (c.curve?.length ? sampleCurve(c.curve, t)[0] : 1)
    case 2:
      return c.scalar * lerp(c.minCurve?.length ? sampleCurve(c.minCurve, t)[0] : 0, c.curve?.length ? sampleCurve(c.curve, t)[0] : 1, r)
    case 3:
      return lerp(c.min, c.scalar, r)
    default:
      return c.scalar
  }
}

/** Colour of a gradient at `t`: colour and alpha keys interpolated separately, as Unity does. */
export function evalGradient(g: Gradient, t: number): [number, number, number, number] {
  const pick = <K extends { t: number }>(keys: readonly K[], f: (k: K) => number[]): number[] => {
    if (!keys.length) return [1, 1, 1, 1]
    if (t <= keys[0].t) return f(keys[0])
    for (let i = 1; i < keys.length; i++) {
      if (t <= keys[i].t) {
        const a = keys[i - 1]
        const b = keys[i]
        const s = b.t > a.t ? (t - a.t) / (b.t - a.t) : 1
        return f(a).map((v, j) => lerp(v, f(b)[j], s))
      }
    }
    return f(keys[keys.length - 1])
  }
  const [r, gr, b] = pick(g.colors, (k) => [k.r, k.g, k.b])
  const [a] = pick(g.alphas, (k) => [k.a])
  return [r, gr, b, a]
}

/** A MinMaxGradient at `t` for a particle's random `r`. */
export function evalMinMaxColor(c: MinMaxColor, t: number, r: number): [number, number, number, number] {
  switch (c.mode) {
    case 1:
      return c.gradient ? evalGradient(c.gradient, t) : c.max
    case 2:
      return c.min.map((v, i) => lerp(v, c.max[i], r)) as [number, number, number, number]
    case 3: {
      const a = c.minGradient ? evalGradient(c.minGradient, t) : c.min
      const b = c.gradient ? evalGradient(c.gradient, t) : c.max
      return a.map((v, i) => lerp(v, b[i], r)) as [number, number, number, number]
    }
    case 4:
      return c.gradient ? evalGradient(c.gradient, r) : c.max
    default:
      return c.max
  }
}
