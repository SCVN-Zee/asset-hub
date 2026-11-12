/**
 * Frames an effect for the Luna camera the way the Unity-style viewer (`FbxViewer.frameModel`) frames it, so that both
 * views show the same part of the effect from the same direction with the same field of view.
 *
 * The viewer works in three.js space, which mirrors Unity's X axis. The result is in Unity space, which is what Luna uses.
 */
import type { LunaCamera, Vec3 } from './protocol'

/** Vertical field of view of the viewer's camera, in degrees. */
export const VIEW_FOV = 45

/** Where the viewer's home view looks from, in units of the framed radius, in three.js space (x, y, z). */
const HOME_OFFSET: Vec3 = [1.6, 0.9, 1.9]

export interface BoxLike {
  min: { x: number; y: number; z: number }
  max: { x: number; y: number; z: number }
}

/** Unity's `Quaternion.LookRotation(forward, up)`. `forward` need not be a unit vector. */
export function lookRotation(forward: Vec3, up: Vec3 = [0, 1, 0]): LunaCamera['rotation'] {
  const length = (v: Vec3) => Math.hypot(v[0], v[1], v[2])
  const unit = (v: Vec3): Vec3 => {
    const l = length(v) || 1
    return [v[0] / l, v[1] / l, v[2] / l]
  }
  const cross = (a: Vec3, b: Vec3): Vec3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
  const f = unit(forward)
  const r = unit(cross(up, f))
  const u = cross(f, r)
  // Rotation matrix with the columns right, up and forward, turned into a quaternion.
  const [m00, m10, m20] = r
  const [m01, m11, m21] = u
  const [m02, m12, m22] = f
  const trace = m00 + m11 + m22
  if (trace > 0) {
    const s = Math.sqrt(trace + 1) * 2
    return [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, s / 4]
  }
  if (m00 > m11 && m00 > m22) {
    const s = Math.sqrt(1 + m00 - m11 - m22) * 2
    return [s / 4, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]
  }
  if (m11 > m22) {
    const s = Math.sqrt(1 + m11 - m00 - m22) * 2
    return [(m01 + m10) / s, s / 4, (m12 + m21) / s, (m02 - m20) / s]
  }
  const s = Math.sqrt(1 + m22 - m00 - m11) * 2
  return [(m02 + m20) / s, (m12 + m21) / s, s / 4, (m10 - m01) / s]
}

/**
 * The camera for an effect whose extent is `box` (in the viewer's three.js space, before the viewer moves the model).
 * An empty box gives no view: the viewer keeps its camera then, and so does Luna.
 */
export function frameEffect(box: BoxLike): LunaCamera | undefined {
  const size: Vec3 = [box.max.x - box.min.x, box.max.y - box.min.y, box.max.z - box.min.z]
  if (!size.every(Number.isFinite) || size.some((v) => v < 0)) return undefined
  const radius = Math.max(Math.hypot(...size) / 2, 1e-3)
  // Three.js space to Unity space: X is mirrored.
  const target: Vec3 = [-(box.min.x + box.max.x) / 2, (box.min.y + box.max.y) / 2, (box.min.z + box.max.z) / 2]
  const offset: Vec3 = [-HOME_OFFSET[0] * radius, HOME_OFFSET[1] * radius, HOME_OFFSET[2] * radius]
  return {
    position: [target[0] + offset[0], target[1] + offset[1], target[2] + offset[2]],
    rotation: lookRotation([-offset[0], -offset[1], -offset[2]]),
    fov: VIEW_FOV,
    near: radius / 100,
    far: radius * 100
  }
}
