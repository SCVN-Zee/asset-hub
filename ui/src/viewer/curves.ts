import type { CurveKey } from '@shared/types'

/** Value of a curve at time `t`: cubic Hermite between keys, held before the first and after the last; infinite tangents step. */
export function sampleCurve(keys: readonly CurveKey[], t: number): number[] {
  if (!keys.length) return []
  if (t <= keys[0].t) return keys[0].v
  const last = keys[keys.length - 1]
  if (t >= last.t) return last.v
  let i = 0
  while (i < keys.length - 2 && keys[i + 1].t < t) i++
  const a = keys[i]
  const b = keys[i + 1]
  const dt = b.t - a.t
  if (dt <= 0) return b.v
  const s = (t - a.t) / dt
  const s2 = s * s
  const s3 = s2 * s
  const h00 = 2 * s3 - 3 * s2 + 1
  const h10 = s3 - 2 * s2 + s
  const h01 = -2 * s3 + 3 * s2
  const h11 = s3 - s2
  return a.v.map((av, c) => {
    const m0 = a.outS[c]
    const m1 = b.inS[c]
    if (!Number.isFinite(m0) || !Number.isFinite(m1)) return av
    return h00 * av + h10 * dt * m0 + h01 * b.v[c] + h11 * dt * m1
  })
}
