import * as THREE from 'three'
import { FBXLoader } from 'three/examples/jsm/loaders/FBXLoader.js'
import type { BuiltinMesh, DependencyReport, ParticleEmitter } from '@shared/types'
import type { LiveContext } from '../components/ModelViewer'
import { mainTexture } from './particleThumb'
import { EffectPlayer, EmitterPlayer, type EmitterLook } from './particlePlayer'
import { portedLook } from './shaderPorts'
import { fetchTextureFile } from './textures'
import { parse, type MaterialData, type MeshData, type ShaderInfo } from '../parse'
import { unwrap } from '../remoteSource'

/** Projects in Gamma colour space, by repo, branch and project root (read once each). */
const gammaProjects = new Map<string, Promise<boolean>>()

/**
 * Whether a project works in Gamma colour space (PlayerSettings m_ActiveColorSpace 0), where Unity
 * computes and blends colours as stored: effects there are drawn that way too. Linear when unknown.
 */
export function isGammaProject(live: LiveContext, background = false): Promise<boolean> {
  const key = `${live.repo}\n${live.branch}\n${live.root}`
  let p = gammaProjects.get(key)
  if (!p) {
    p = window.api.assets
      .readFile(live.repo, live.branch, `${live.root}ProjectSettings/ProjectSettings.asset`, background)
      .then((r) => (r.ok ? /^\s*m_ActiveColorSpace: 0\s*$/m.test(new TextDecoder().decode(r.data)) : false), () => false)
    gammaProjects.set(key, p)
  }
  return p
}

/** Mesh particles from models larger than this are drawn as billboards. */
const MAX_MESH_BYTES = 8 * 1024 * 1024

/**
 * Effect packs with their own shaders often keep the mask in a colour channel
 * (a "gas alpha" from green, a flame drawn white on black) and leave the
 * file's alpha solid. When the alpha is opaque everywhere but the edges are
 * black, the alpha is taken from the brightest channel. Undefined otherwise:
 * the file's alpha is real.
 */
function maskFromColour(image: HTMLImageElement): HTMLCanvasElement | undefined {
  const w = image.naturalWidth
  const h = image.naturalHeight
  if (!w || !h) return undefined
  const canvas = document.createElement('canvas')
  canvas.width = w
  canvas.height = h
  const g = canvas.getContext('2d', { willReadFrequently: true })!
  g.drawImage(image, 0, 0)
  const pixels = g.getImageData(0, 0, w, h)
  const d = pixels.data
  let edge = 0
  let edges = 0
  for (let i = 0; i < d.length; i += 4) {
    if (d[i + 3] < 250) return undefined
    const x = (i / 4) % w
    const y = Math.floor(i / 4 / w)
    if (x === 0 || y === 0 || x === w - 1 || y === h - 1) {
      edge += Math.max(d[i], d[i + 1], d[i + 2])
      edges++
    }
  }
  if (edge / edges > 24) return undefined
  for (let i = 0; i < d.length; i += 4) d[i + 3] = Math.max(d[i], d[i + 1], d[i + 2])
  g.putImageData(pixels, 0, 0)
  return canvas
}

/**
 * A mesh Unity saved as YAML (effect packs' waves and rings), turned to match meshes from an
 * FBX: those arrive mirrored along X, which the particle system's own mirroring undoes.
 */
function nativeMesh(m: MeshData | undefined): THREE.BufferGeometry | undefined {
  if (!m) return undefined
  const flip = (a: Float32Array) => {
    const out = a.slice()
    for (let i = 0; i < out.length; i += 3) out[i] = -out[i]
    return out
  }
  const g = new THREE.BufferGeometry()
  g.setAttribute('position', new THREE.BufferAttribute(flip(m.positions), 3))
  if (m.normals) g.setAttribute('normal', new THREE.BufferAttribute(flip(m.normals), 3))
  if (m.uvs) g.setAttribute('uv', new THREE.BufferAttribute(m.uvs, 2))
  // Mirroring turns triangles around: wind them back.
  const idx = m.indices.slice()
  for (let i = 0; i + 2 < idx.length; i += 3) [idx[i + 1], idx[i + 2]] = [idx[i + 2], idx[i + 1]]
  g.setIndex(new THREE.BufferAttribute(idx, 1))
  return g
}

/** Unity's own meshes at their sizes (Quad and Plane facing along Z and Y like Unity's). */
function builtinGeometry(mesh: BuiltinMesh): THREE.BufferGeometry {
  switch (mesh) {
    case 'quad':
      return new THREE.PlaneGeometry(1, 1)
    case 'plane':
      return new THREE.PlaneGeometry(10, 10).rotateX(-Math.PI / 2)
    case 'cube':
      return new THREE.BoxGeometry(1, 1, 1)
    case 'sphere':
      return new THREE.SphereGeometry(0.5, 24, 16)
    case 'cylinder':
      return new THREE.CylinderGeometry(0.5, 0.5, 2, 24)
    case 'capsule':
      return new THREE.CapsuleGeometry(0.5, 1, 8, 16)
  }
}

/** Unity's legacy particle shaders blend one fixed way, whatever numbers their materials kept from another shader. */
const FIXED_BLEND = /^(Legacy Shaders|Mobile)\/Particles\//

/** Blend from the shader's name when the material does not say (legacy "Particles/Additive"…). */
function blendFromShader(label: string | undefined): EmitterLook['blend'] {
  if (!label) return 'alpha'
  if (/add/i.test(label)) return 'additive'
  if (/premultipl/i.test(label)) return 'premultiply'
  if (/multipl/i.test(label)) return 'multiply'
  return 'alpha'
}

/**
 * Puts a player on every emitter of a built prefab: each gets its material's
 * main texture, colour and blend. Particles appear under the emitters' objects
 * as soon as the returned player is updated.
 */
export async function buildEffect(live: LiveContext, report: DependencyReport, anchors: ReadonlyArray<{ emitter: ParticleEmitter & { prefab?: string }; object: THREE.Object3D }>, background = false): Promise<EffectPlayer> {
  const byGuid = new Map(report.materials.flatMap((m) => (m.guid ? [[m.guid, m] as const] : [])))
  const gamma = await isGammaProject(live, background)
  const textures = new Map<string, Promise<THREE.Texture | undefined>>()
  let maxSide = 0
  /** `alphaInColour`: a custom shader blending by alpha, which may take it from a colour channel (see `maskFromColour`). */
  const loadTexture = (path: string, alphaInColour = false) => {
    let p = textures.get(`${path}#${alphaInColour}`)
    if (!p) {
      p = fetchTextureFile(live, path, background).then(async (file) => {
        if (!file) return undefined
        try {
          const tex = await new THREE.TextureLoader().loadAsync(file.url)
          if (alphaInColour) {
            const masked = maskFromColour(tex.image as HTMLImageElement)
            if (masked) tex.image = masked
          }
          tex.colorSpace = gamma ? THREE.NoColorSpace : THREE.SRGBColorSpace
          const img = tex.image as { width?: number; height?: number } | undefined
          maxSide = Math.max(maxSide, img?.width ?? 0, img?.height ?? 0)
          return tex
        } catch {
          return undefined
        } finally {
          URL.revokeObjectURL(file.url)
        }
      })
      textures.set(`${path}#${alphaInColour}`, p)
    }
    return p
  }
  const mats = new Map<string, Promise<MaterialData | undefined>>()
  /** A .mat file's look and properties (read once; undefined when it cannot be read). */
  const loadMat = (path: string) => {
    let p = mats.get(path)
    if (!p) {
      p = parse.material(live, path, background).catch(() => undefined)
      mats.set(path, p)
    }
    return p
  }
  /** A texture for a ported shader: wrapped and decoded as Unity imports it (scrolling and noise maps repeat; data maps are not sRGB). */
  const loadPortTexture = (path: string) => {
    let p = textures.get(`${path}#port`)
    if (!p) {
      p = Promise.all([
        fetchTextureFile(live, path, background),
        parse.textureInfo(live, path, undefined, background).then((info) => info.import ?? undefined, () => undefined)
      ]).then(async ([file, meta]) => {
        if (!file) return undefined
        try {
          const tex = await new THREE.TextureLoader().loadAsync(file.url)
          const wrap = (w: number | undefined) => (w === 1 ? THREE.ClampToEdgeWrapping : w === 2 || w === 3 ? THREE.MirroredRepeatWrapping : THREE.RepeatWrapping)
          tex.wrapS = wrap(meta?.wrapU)
          tex.wrapT = wrap(meta?.wrapV)
          tex.colorSpace = meta?.sRGB === false || gamma ? THREE.NoColorSpace : THREE.SRGBColorSpace
          const img = tex.image as { width?: number; height?: number } | undefined
          maxSide = Math.max(maxSide, img?.width ?? 0, img?.height ?? 0)
          return tex
        } catch {
          return undefined
        } finally {
          URL.revokeObjectURL(file.url)
        }
      })
      textures.set(`${path}#port`, p)
    }
    return p
  }

  const shaders = new Map<string, Promise<{ blend?: EmitterLook['blend']; textures: string[]; shader: ShaderInfo } | undefined>>()
  /** What a project shader tells: the blend its code fixes, if it does, and the texture slots it reads. */
  const loadShader = (path: string) => {
    let p = shaders.get(path)
    if (!p) {
      p = parse.shaderInfo(live, path, background).then(
        (shader) => ({ blend: shader.blend ?? undefined, textures: shader.textures, shader }),
        () => undefined
      )
      shaders.set(path, p)
    }
    return p
  }

  const meshes = new Map<string, Promise<THREE.BufferGeometry | undefined>>()
  /** The mesh of a model a particle system names (by fileID; else its first), in Unity's import units. */
  const loadMesh = (path: string, fileID?: string) => {
    let p = meshes.get(`${path}#${fileID}`)
    if (!p) {
      p = (async () => {
        try {
          const read = (f: string) => window.api.assets.readFile(live.repo, live.branch, f, background)
          if (!/\.fbx$/i.test(path)) return nativeMesh(await parse.unityMesh(live, path, background))
          const [bytes, info] = await Promise.all([read(path).then(unwrap), parse.modelInfo(live, path, background)])
          if (bytes.byteLength > MAX_MESH_BYTES) return undefined
          const root = new FBXLoader().parse(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer, '')
          const all: THREE.Mesh[] = []
          root.traverse((o) => (o as THREE.Mesh).isMesh && all.push(o as THREE.Mesh))
          const named = fileID ? await parse.meshForFileId(all.map((m) => m.name || m.parent?.name || ''), fileID) : undefined
          const mesh = all[Math.max(0, named ?? 0)]
          if (!mesh) return undefined
          const { globalScale, useFileScale } = info.importScale
          const unit = Number(root.userData.unitScaleFactor) || 1
          const k = (useFileScale ? unit / 100 : 1) * globalScale
          return mesh.geometry.clone().scale(k, k, k)
        } catch {
          return undefined
        }
      })()
      meshes.set(`${path}#${fileID}`, p)
    }
    return p
  }
  /** Texture, colour and blend of one material. */
  const lookOf = async (guid: string | undefined): Promise<Omit<EmitterLook, 'mesh' | 'trail'>> => {
    const material = guid ? byGuid.get(guid) : undefined
    const shader = material?.shader
    const info = shader?.kind === 'project' && shader.path ? await loadShader(shader.path) : undefined
    const coded = info?.blend
    const texPath = mainTexture(material, info?.textures)
    const matData = material?.path ? await loadMat(material.path) : undefined
    const look = matData?.look
    const c = look?.color
    // A blend fixed by the shader wins over the material's numbers, which may be left from another shader.
    const fixed = shader && FIXED_BLEND.test(shader.label) ? blendFromShader(shader.label) : coded
    const blend = fixed ?? look?.blend ?? blendFromShader(shader?.label)
    const custom = !!shader && shader.kind !== 'builtin' && (blend === 'alpha' || blend === 'premultiply')
    const texture = texPath ? await loadTexture(texPath, custom) : undefined
    // A shader we have a port of draws exactly; the rest above stays for mesh particles, which it does not cover.
    const ported = info && material && matData ? await portedLook(material, info.shader, matData, { texture: loadPortTexture, embedded: (path, fileID) => parse.embeddedTexture(live, path, fileID, background), gamma }).catch(() => undefined) : undefined
    const legacy = shader?.kind !== 'builtin' ? undefined : /Alpha Blended Premultiply/.test(shader.label) ? 'premultiply' : /Additive \(Soft\)/.test(shader.label) ? 'soft' : undefined
    return { texture, blend, tint: c ? [c.r, c.g, c.b, c.a] : [1, 1, 1, 1], ...(ported ? { custom: ported } : {}), ...(legacy ? { legacy } : {}), ...(gamma ? { gamma } : {}) }
  }

  const players = await Promise.all(
    anchors.map(async ({ emitter, object }) => {
      const meshPath = emitter.renderMode === 'mesh' && emitter.mesh ? report.guidPaths?.[emitter.mesh] : undefined
      const [look, trail, mesh] = await Promise.all([
        lookOf(emitter.material),
        emitter.sim.trail ? lookOf(emitter.trailMaterial ?? emitter.material) : undefined,
        meshPath && /\.(fbx|mesh|asset)$/i.test(meshPath) ? loadMesh(meshPath, emitter.meshFileID) : emitter.renderMode === 'mesh' && emitter.builtinMesh ? builtinGeometry(emitter.builtinMesh) : undefined
      ])
      const player = new EmitterPlayer(emitter, { ...look, trail, mesh })
      object.add(player.mesh)
      return player
    })
  )

  // Sub emitters: the system each one names, in the same prefab — the one under the parent's object when it is placed twice.
  for (const [i, parent] of players.entries()) {
    for (const sub of parent.emitter.sim.subEmitters ?? []) {
      const candidates = players.filter((p) => p !== parent && p.emitter.id === sub.id && p.emitter.prefab === parent.emitter.prefab)
      const under = candidates.find((p) => {
        let o: THREE.Object3D | null = anchors[players.indexOf(p)].object
        for (; o; o = o.parent) if (o === anchors[i].object) return true
        return false
      })
      const child = under ?? candidates[0]
      if (!child) continue
      child.isSub = true
      parent.subs.push({ player: child, type: sub.type, inherit: sub.inherit, probability: sub.probability })
    }
  }
  // Sub emitters start empty: they wait for their parents' particles.
  for (const p of players) if (p.isSub) p.restart()
  const effect = new EffectPlayer(players)
  effect.maxTextureSize = maxSide
  const dispose = effect.dispose.bind(effect)
  effect.dispose = () => {
    dispose()
    for (const p of textures.values()) void p.then((t) => t?.dispose())
    for (const p of meshes.values()) void p.then((g) => g?.dispose())
  }
  return effect
}
