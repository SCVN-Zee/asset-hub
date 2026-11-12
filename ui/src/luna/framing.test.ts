import { describe, expect, it } from 'vitest'
import { frameEffect, lookRotation, VIEW_FOV } from './framing'

type Q = readonly [number, number, number, number]
/** Rotates a vector by a unit quaternion. */
function rotate(q: Q, v: readonly [number, number, number]): [number, number, number] {
  const [x, y, z, w] = q
  const [vx, vy, vz] = v
  const tx = 2 * (y * vz - z * vy)
  const ty = 2 * (z * vx - x * vz)
  const tz = 2 * (x * vy - y * vx)
  return [vx + w * tx + (y * tz - z * ty), vy + w * ty + (z * tx - x * tz), vz + w * tz + (x * ty - y * tx)]
}
const close = (a: readonly number[], b: readonly number[]) => a.forEach((v, i) => expect(v).toBeCloseTo(b[i], 6))

describe('lookRotation', () => {
  it('is the identity when looking along +Z', () => {
    close(lookRotation([0, 0, 1]), [0, 0, 0, 1])
  })

  it('turns +Z onto the forward direction and keeps +Y up for a level view', () => {
    const forward: [number, number, number] = [1, -0.5, 2]
    const q = lookRotation(forward)
    const len = Math.hypot(...forward)
    close(rotate(q, [0, 0, 1]), forward.map((v) => v / len))
    // The camera's up has no sideways part: its X-Z projection is not turned off the view direction.
    const up = rotate(q, [0, 1, 0])
    expect(up[1]).toBeGreaterThan(0)
  })

  it('gives a unit quaternion on every branch of the conversion', () => {
    for (const f of [[0, 0, -1], [1, 0, 0], [-1, 0, 0], [0, 0.0001, -1], [0.3, -2, 0.1], [0, -1, 0.0001]] as const) {
      const q = lookRotation(f)
      expect(Math.hypot(...q)).toBeCloseTo(1, 6)
      const len = Math.hypot(...f)
      close(rotate(q, [0, 0, 1]), f.map((v) => v / len))
    }
  })
})

describe('frameEffect', () => {
  const box = { min: { x: -1, y: 0, z: -1 }, max: { x: 3, y: 2, z: 1 } }

  it('stands at the viewer home distance and direction, mirrored into Unity space', () => {
    const view = frameEffect(box)!
    const radius = Math.hypot(4, 2, 2) / 2
    // Box centre (1, 1, 0) in three.js space is (-1, 1, 0) in Unity space.
    const distance = Math.hypot(view.position[0] - -1, view.position[1] - 1, view.position[2] - 0)
    expect(distance).toBeCloseTo(radius * Math.hypot(1.6, 0.9, 1.9), 6)
    // The camera stands on the -X side in Unity space (the +X side in three.js) and in front of the effect.
    expect(view.position[0]).toBeLessThan(-1)
    expect(view.position[1]).toBeGreaterThan(1)
    expect(view.position[2]).toBeGreaterThan(0)
  })

  it('uses the viewer field of view and clip planes', () => {
    const view = frameEffect(box)!
    expect(view.fov).toBe(VIEW_FOV)
    expect(view.near).toBeCloseTo(Math.hypot(4, 2, 2) / 2 / 100, 9)
    expect(view.far).toBeCloseTo((Math.hypot(4, 2, 2) / 2) * 100, 9)
  })

  it('looks straight at the centre', () => {
    const view = frameEffect(box)!
    const forward = rotate(view.rotation, [0, 0, 1])
    const toCentre = [-1 - view.position[0], 1 - view.position[1], 0 - view.position[2]]
    const length = Math.hypot(...toCentre)
    close(forward, toCentre.map((v) => v / length))
  })

  it('has no view for an empty box', () => {
    const empty = { min: { x: Infinity, y: Infinity, z: Infinity }, max: { x: -Infinity, y: -Infinity, z: -Infinity } }
    expect(frameEffect(empty)).toBeUndefined()
  })

  it('keeps a point-sized box valid', () => {
    const view = frameEffect({ min: { x: 0, y: 0, z: 0 }, max: { x: 0, y: 0, z: 0 } })!
    expect(view.near).toBeGreaterThan(0)
    expect(view.far).toBeGreaterThan(view.near)
  })
})
