import * as THREE from 'three'
import type { MaterialProps, MaterialRef } from '@shared/types'
import { BUILTIN_SHADER_GUIDS } from '../../bindings'
import type { EmbeddedTexture, MaterialData, ShaderInfo } from '../../parse'
import type { CustomLook } from '../particlePlayer'
import { allIn1Fragment, ALLIN1_SHADER, ALLIN1_TEXTURES, UNITY_BLEND_FACTORS } from './allIn1Vfx'
import { KLAUS_FRAGMENT, KLAUS_SHADER } from './klaus'

/** Gamma to linear, as Unity hands material colours to shaders in a linear project. */
const linear = (v: number) => (v > 0 ? Math.pow(v, 2.2) : v)

/** Unity's default for a texture slot left empty ("white"). */
let white: THREE.Texture | undefined
const whiteTexture = () => (white ??= Object.assign(new THREE.DataTexture(new Uint8Array([255, 255, 255, 255]), 1, 1), { needsUpdate: true }))

/** Unity's Default-Particle (a built-in texture, fileID 10300): a soft round dot, as the generic particle shader draws one. */
let dot: THREE.Texture | undefined
function defaultParticle(): THREE.Texture {
  if (dot) return dot
  const size = 64
  const data = new Uint8Array(size * size * 4)
  for (let y = 0; y < size; y++)
    for (let x = 0; x < size; x++) {
      const d = Math.hypot((x + 0.5) / size - 0.5, (y + 0.5) / size - 0.5)
      const t = Math.min(1, Math.max(0, (d - 0.15) / 0.35))
      data.set([255, 255, 255, Math.round(255 * (1 - t * t * (3 - 2 * t)))], (y * size + x) * 4)
    }
  dot = new THREE.DataTexture(data, size, size)
  dot.needsUpdate = true
  return dot
}

/** How a ported shader gets files: textures as their import settings say, and the textures saved inside other assets. */
export interface PortFiles {
  texture(path: string): Promise<THREE.Texture | undefined>
  /** A texture saved inside an asset (a .mat or .asset), by fileID. */
  embedded(path: string, fileID: string): Promise<EmbeddedTexture | undefined>
  /** The project works in Gamma colour space: colours stay as stored. */
  gamma?: boolean
}

/** The texture in one slot: a texture file, one saved inside another asset or the material itself, Unity's own particle dot, or white. */
async function slotTexture(material: MaterialRef, props: MaterialProps, slot: string, files: PortFiles): Promise<THREE.Texture> {
  const ref = props.refs[slot]
  if (!ref) return whiteTexture()
  if (ref.guid && (BUILTIN_SHADER_GUIDS as readonly string[]).includes(ref.guid)) return ref.fileID === '10300' ? defaultParticle() : whiteTexture()
  // No GUID: a texture saved inside the material (AllIn1Vfx keeps its gradients there).
  const path = ref.guid ? material.textures.find((t) => t.prop === slot)?.path : material.path
  if (!path) return whiteTexture()
  if (ref.fileID !== '2800000' && /\.(mat|asset)$/i.test(path)) {
    const tex = await files.embedded(path, ref.fileID).catch(() => undefined)
    if (!tex) return whiteTexture()
    const t = new THREE.DataTexture(tex.rgba, tex.width, tex.height)
    t.colorSpace = tex.linear || files.gamma ? THREE.NoColorSpace : THREE.SRGBColorSpace
    t.wrapS = t.wrapT = tex.clamp ? THREE.ClampToEdgeWrapping : THREE.RepeatWrapping
    t.magFilter = t.minFilter = tex.point ? THREE.NearestFilter : THREE.LinearFilter
    t.needsUpdate = true
    return t
  }
  return (await files.texture(path)) ?? whiteTexture()
}

/**
 * A look drawn by a ported shader, when the material's shader is one we have a
 * port of; undefined otherwise (the generic texture × tint look is used).
 * `texture` loads a slot's texture as its import settings say (wrap, sRGB).
 */
export async function portedLook(material: MaterialRef, shader: ShaderInfo, mat: MaterialData, files: PortFiles): Promise<CustomLook | undefined> {
  const name = shader.name ?? ''
  if (KLAUS_SHADER.test(name)) return klausLook(material, shader, mat, files)
  if (!ALLIN1_SHADER.test(name)) return undefined
  const props = mat.props
  const defaults = shader.defaults
  const floats = { ...defaults.floats, ...props.floats }
  const colors = { ...defaults.colors, ...props.colors }
  const uniforms: Record<string, THREE.IUniform> = {}
  for (const [k, v] of Object.entries(floats)) uniforms[k] = { value: v }
  for (const [k, c] of Object.entries(colors)) uniforms[k] = { value: files.gamma ? new THREE.Vector4(c.r, c.g, c.b, c.a) : new THREE.Vector4(linear(c.r), linear(c.g), linear(c.b), c.a) }
  // Only the slots the material's features read are downloaded.
  const used = new Set(['_MainTex', ...ALLIN1_TEXTURES.filter((slot) => usesSlot(slot, props.keywords))])
  await Promise.all(
    ALLIN1_TEXTURES.map(async (slot) => {
      uniforms[slot] = { value: used.has(slot) ? await slotTexture(material, props, slot, files) : whiteTexture() }
      const st = props.st[slot] ?? [1, 1, 0, 0]
      uniforms[`${slot}_ST`] = { value: new THREE.Vector4(...st) }
    })
  )
  const main = uniforms._MainTex.value as THREE.Texture
  const img = main.image as { width?: number; height?: number } | undefined
  const w = img?.width || 1
  const h = img?.height || 1
  uniforms._MainTex_TexelSize = { value: new THREE.Vector4(1 / w, 1 / h, w, h) }
  return {
    fragmentShader: allIn1Fragment(props.keywords),
    uniforms,
    blendSrc: UNITY_BLEND_FACTORS[floats._SrcMode ?? 5] as THREE.BlendingSrcFactor,
    blendDst: UNITY_BLEND_FACTORS[floats._DstMode ?? 10] as THREE.BlendingDstFactor,
    depthWrite: floats._ZWrite === 1,
    // Unity's Cull: 0 off, 1 front (the back faces are drawn), 2 back.
    side: floats._CullingOption === 1 ? THREE.BackSide : floats._CullingOption === 2 ? THREE.FrontSide : THREE.DoubleSide
  }
}

async function klausLook(material: MaterialRef, shader: ShaderInfo, mat: MaterialData, files: PortFiles): Promise<CustomLook> {
  const props = mat.props
  const floats = { ...shader.defaults.floats, ...props.floats }
  const additive = /_add$/.test(shader.name ?? '')
  return {
    fragmentShader: KLAUS_FRAGMENT,
    uniforms: {
      _Main_Tex: { value: await slotTexture(material, props, '_Main_Tex', files) },
      _Main_Tex_ST: { value: new THREE.Vector4(...(props.st._Main_Tex ?? [1, 1, 0, 0])) },
      _Emission_Power: { value: floats._Emission_Power ?? 1 },
      uAdditive: { value: additive ? 1 : 0 }
    },
    blendSrc: THREE.SrcAlphaFactor,
    blendDst: additive ? THREE.OneFactor : THREE.OneMinusSrcAlphaFactor
  }
}

/** Whether a material's keywords make the shader read a slot. */
function usesSlot(slot: string, keywords: readonly string[]): boolean {
  const on = (k: string) => keywords.includes(k)
  switch (slot) {
    case '_ShapeDistortTex':
      return on('SHAPE1DISTORT_ON')
    case '_Shape2Tex':
      return on('SHAPE2_ON')
    case '_Shape2DistortTex':
      return on('SHAPE2_ON') && on('SHAPE2DISTORT_ON')
    case '_Shape3Tex':
      return on('SHAPE3_ON')
    case '_Shape3DistortTex':
      return on('SHAPE3_ON') && on('SHAPE3DISTORT_ON')
    case '_Shape1MaskTex':
      return on('SHAPE1MASK_ON')
    case '_DistortTex':
      return on('DISTORT_ON')
    case '_TrailWidthGradient':
      return on('TRAILWIDTH_ON')
    case '_FadeTex':
      return on('FADE_ON')
    case '_FadeBurnTex':
      return on('FADE_ON') && on('FADEBURN_ON')
    case '_ColorRampTex':
      return on('COLORRAMP_ON') && !on('COLORRAMPGRAD_ON')
    case '_ColorRampTexGradient':
      return on('COLORRAMP_ON') && on('COLORRAMPGRAD_ON')
    case '_GlowTex':
      return on('GLOW_ON') && on('GLOWTEX_ON')
    case '_MaskTex':
      return on('MASK_ON')
    default:
      return false
  }
}
