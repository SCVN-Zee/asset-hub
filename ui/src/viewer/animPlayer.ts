import * as THREE from 'three'
import { FBXLoader } from 'three/examples/jsm/loaders/FBXLoader.js'
import { clone as cloneWithBones } from 'three/examples/jsm/utils/SkeletonUtils.js'
import { BLANK_TEXTURE } from './FbxViewer'
import type { AnimClipInfo } from '@shared/types'
import { parse } from '../parse'
import { sampleCurve } from './curves'
import type { LiveContext } from '../components/ModelViewer'
import { unwrap } from '../remoteSource'

/** Rigs bigger than this are not downloaded to try a clip on. */
export const MAX_RIG_BYTES = 30 * 1024 * 1024
/** The animated root's name in tracks ("" paths in Unity). */
const ROOT = '__clipRoot'

const sanitize = (name: string) => THREE.PropertyBinding.sanitizeNodeName(name)
const leaf = (path: string) => path.split('/').pop() ?? ''

/** A model loaded to play clips on: in file units, with Unity's import scale to convert clip positions. */
export interface Rig {
  path: string
  root: THREE.Group
  /** Unity units per file unit (UnitScaleFactor / 100 × globalScale). */
  factor: number
  bytes: number
}

/** The last few rigs parsed: clips of one character share it, so each FBX is read once. */
const rigs = new Map<string, Promise<Rig>>()
const RIG_CACHE = 4

/** A rig to show: a copy of the parsed model (a viewer disposes what it shows). */
export function loadRig(live: LiveContext, path: string, background = false): Promise<Rig> {
  const key = `${live.repo}@${live.branch}:${path}`
  let parsed = rigs.get(key)
  if (!parsed) {
    parsed = parseRig(live, path, background)
    rigs.set(key, parsed)
    parsed.catch(() => rigs.delete(key))
    if (rigs.size > RIG_CACHE) rigs.delete(rigs.keys().next().value!)
  }
  return parsed.then((r) => {
    const root = cloneWithBones(r.root) as THREE.Group
    root.name = ROOT
    return { ...r, root }
  })
}

async function parseRig(live: LiveContext, path: string, background: boolean): Promise<Rig> {
  const [bytes, model] = await Promise.all([window.api.assets.readFile(live.repo, live.branch, path, background).then(unwrap), parse.modelInfo(live, path, background)])
  if (bytes.byteLength > MAX_RIG_BYTES) throw new Error(`${path} is too large to try as a rig`)
  const manager = new THREE.LoadingManager()
  manager.setURLModifier(() => BLANK_TEXTURE)
  const root = new FBXLoader(manager).parse(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer, '')
  root.name = ROOT
  const { globalScale, useFileScale } = model.importScale
  const unit = Number(root.userData.unitScaleFactor) || 1
  return { path, root, factor: (useFileScale ? unit / 100 : 1) * globalScale, bytes: bytes.byteLength }
}

/** Share of the clip's animated objects the rig has, by name (0–1). */
export function rigCoverage(info: AnimClipInfo, root: THREE.Object3D): number {
  const names = new Set<string>()
  root.traverse((o) => names.add(o.name))
  const wanted = [...new Set(info.transformCurves.map((c) => c.path).filter(Boolean).map((p) => sanitize(leaf(p))))]
  if (!wanted.length) return info.transformCurves.length ? 1 : 0
  return wanted.filter((n) => names.has(n)).length / wanted.length
}

/**
 * The clip as a three.js clip on `rig`: curves sampled at the clip's rate
 * (at least 30 fps), mirrored from Unity's left-handed axes, positions back
 * in file units. Objects the rig does not have are left out.
 */
export function toThreeClip(info: AnimClipInfo, rig: Rig): THREE.AnimationClip {
  const fps = Math.min(60, Math.max(30, info.sampleRate))
  const frames = Math.max(2, Math.ceil(info.length * fps) + 1)
  const times = Float32Array.from({ length: frames }, (_, i) => Math.min(info.length, i / fps))
  const names = new Set<string>()
  rig.root.traverse((o) => names.add(o.name))
  const tracks: THREE.KeyframeTrack[] = []
  const euler = new THREE.Euler()
  const q = new THREE.Quaternion()
  for (const curve of info.transformCurves) {
    const name = curve.path ? sanitize(leaf(curve.path)) : ROOT
    if (!names.has(name)) continue
    const at = (t: number) => sampleCurve(curve.keys, t)
    // Root motion would carry the model out of the framed view: left out.
    if (curve.kind === 'position' && !curve.path) continue
    if (curve.kind === 'position') {
      const f = rig.factor || 1
      tracks.push(new THREE.VectorKeyframeTrack(`${name}.position`, times, [...times].flatMap((t) => { const [x, y, z] = at(t); return [-x / f, y / f, z / f] })))
    } else if (curve.kind === 'scale') {
      tracks.push(new THREE.VectorKeyframeTrack(`${name}.scale`, times, [...times].flatMap((t) => at(t))))
    } else {
      const values: number[] = []
      let prev: THREE.Quaternion | undefined
      for (const t of times) {
        const v = at(t)
        if (curve.kind === 'rotation') q.set(v[0], v[1], v[2], v[3]).normalize()
        // Unity's Euler angles turn Z, then X, then Y: three's 'YXZ'.
        else q.setFromEuler(euler.set(THREE.MathUtils.degToRad(v[0]), THREE.MathUtils.degToRad(v[1]), THREE.MathUtils.degToRad(v[2]), 'YXZ'))
        const m = new THREE.Quaternion(q.x, -q.y, -q.z, q.w)
        // Keep neighbouring keys on the same hemisphere so interpolation takes the short way.
        if (prev && prev.dot(m) < 0) m.set(-m.x, -m.y, -m.z, -m.w)
        values.push(m.x, m.y, m.z, m.w)
        prev = m
      }
      tracks.push(new THREE.QuaternionKeyframeTrack(`${name}.quaternion`, times, values))
    }
  }
  return new THREE.AnimationClip(info.name || 'clip', info.length || -1, tracks)
}

/** Where in a clip its thumbnail pose is taken (share of its length): past the wind-up, into the motion. */
export const THUMB_POSE = 0.4

/** Rigs tried automatically before giving up (each is a download). */
export const AUTO_TRIES = 4
/**
 * A rig with this share of the clip's objects is taken without trying more:
 * the first candidates are the likeliest models; a generic rig further down
 * may match more bones yet be the wrong character.
 */
export const GOOD_ENOUGH = 0.5

/** The first likely model whose rig has most of the clip's objects; the coverage of every rig tried. */
export async function pickRig(
  live: LiveContext,
  info: AnimClipInfo,
  candidates: readonly string[],
  options: { tries?: number; background?: boolean; isCancelled?(): boolean; onTried?(path: string, coverage: number): void } = {}
): Promise<{ rig: Rig; coverage: number } | undefined> {
  let best: { rig: Rig; coverage: number } | undefined
  for (const p of candidates.slice(0, options.tries ?? AUTO_TRIES)) {
    if (options.isCancelled?.()) return undefined
    try {
      const rig = await loadRig(live, p, options.background)
      const coverage = rigCoverage(info, rig.root)
      options.onTried?.(p, coverage)
      if (!best || coverage > best.coverage) best = { rig, coverage }
      if (coverage >= GOOD_ENOUGH) break
    } catch {
      options.onTried?.(p, 0)
    }
  }
  return best && best.coverage > 0 ? best : undefined
}

/** A model picked by hand is remembered for the clip's folder: clips side by side animate the same character. */
const rigKey = (live: LiveContext, animPath: string) => `assethub.rig:${live.repo}:${animPath.slice(0, animPath.lastIndexOf('/'))}`

export function rememberedRig(live: LiveContext, animPath: string): string | undefined {
  try {
    return localStorage.getItem(rigKey(live, animPath)) ?? undefined
  } catch {
    return undefined
  }
}

export function rememberRig(live: LiveContext, animPath: string, rigPath: string): void {
  try {
    localStorage.setItem(rigKey(live, animPath), rigPath)
  } catch {
    // Not remembered: the choice still applies to this clip.
  }
}

/** Candidates to try, the model remembered for the clip's folder first. */
export function rigOrder(live: LiveContext, animPath: string, candidates: readonly string[]): string[] {
  const remembered = rememberedRig(live, animPath)
  return remembered ? [remembered, ...candidates.filter((c) => c !== remembered)] : [...candidates]
}
