/**
 * Turns a material we parsed (shader, colours, textures, tiling, floats, keywords) into the serialisable material the
 * Luna host draws (`LunaMaterialSpec`). Pure: the caller loads and decodes the textures and hands them in, so this
 * module reads no files and the host needs no Unity parsing.
 *
 * The shader modules the ids refer to are in `host/shaders/`. A shader without a module maps to `shader: null`, which
 * the host draws with its fallback shader and reports as `unsupported`.
 */
import type { MaterialLook, MaterialProps, ShaderKind, TextureImport } from '@shared/types'
import { BUILTIN_SHADER_GUIDS } from '../bindings'
import { ALLIN1_SHADER, ALLIN1_TEXTURES } from '../viewer/shaderPorts/allIn1Vfx'
import { KLAUS_SHADER } from '../viewer/shaderPorts/klaus'
import type { LunaMaterialSpec, LunaSampler, LunaShaderState, LunaTextureSpec, Rgba } from './protocol'

// Unity's BlendMode numbers, as the material inspector writes them.
const ONE = 1
const SRC_ALPHA = 5
const ONE_MINUS_SRC_ALPHA = 10
const ZERO = 0
const DST_COLOR = 2

const WHITE: Rgba = [1, 1, 1, 1]
const IDENTITY_ST: Rgba = [1, 1, 0, 0]
/** Slots a legacy or mobile particle shader's own colour map can be in, in the order the Unity-style player reads them. */
const MAIN_SLOTS = ['_MainTex', '_BaseMap', '_BaseColorMap', '_Texture', '_TintTex'] as const

/** Unity shader names that a host module draws, by module id. Keep in step with `host/shaders/` (a test checks it). */
const BUILTIN_SHADERS: Readonly<Record<string, string>> = {
  'Legacy Shaders/Particles/Additive': 'legacy-additive',
  'Legacy Shaders/Particles/Additive (Soft)': 'legacy-additive-soft',
  'Legacy Shaders/Particles/Alpha Blended': 'legacy-alpha-blended',
  'Legacy Shaders/Particles/Alpha Blended Premultiply': 'legacy-alpha-blended-premultiply',
  'Legacy Shaders/Particles/Multiply': 'legacy-multiply',
  'Mobile/Particles/Additive': 'mobile-additive',
  'Mobile/Particles/Alpha Blended': 'mobile-alpha-blended',
  'Universal Render Pipeline/Particles/Unlit': 'urp-particles-unlit'
}

export interface MaterialInput {
  shader: { label: string; kind: ShaderKind } | undefined
  /** The name a project shader declares (`ShaderInfo.name`). Preferred over the label when there is one. */
  shaderName?: string
  props: MaterialProps
  look?: MaterialLook
  /** What the shader sets for numbers the material leaves out (`ShaderInfo.defaults`). */
  defaults?: Pick<MaterialProps, 'floats' | 'colors'>
  /** Decoded textures by slot. A slot that is missing, or null, draws white. */
  textures: Readonly<Record<string, LunaTextureSpec | null | undefined>>
}

const unityToRgba = (c: { r: number; g: number; b: number; a: number }): Rgba => [c.r, c.g, c.b, c.a]

// ---- textures ---------------------------------------------------------------------------------------------------

const wrapOf = (mode: number | undefined): LunaSampler['wrapU'] => (mode === 1 ? 'clamp' : mode === 2 || mode === 3 ? 'mirror' : 'repeat')

/**
 * Sampler fields from a texture's import settings. Wrap 0 (and -1, the default) repeats. In a Gamma project nothing is
 * decoded, so `srgb` is false. Unity's filter mode is not part of the parsed import, so it is bilinear unless given.
 */
export function samplerFromImport(imported: TextureImport | null | undefined, gamma: boolean, filter: LunaSampler['filter'] = 'bilinear'): LunaSampler {
  return {
    wrapU: wrapOf(imported?.wrapU),
    wrapV: wrapOf(imported?.wrapV),
    filter,
    srgb: imported?.sRGB !== false && !gamma,
    mipmaps: imported?.mipmaps ?? true
  }
}

/** Sampler fields for a texture saved inside another asset (see `EmbeddedTexture`). */
export function samplerFromEmbedded(texture: { linear: boolean; clamp: boolean; point: boolean }, gamma: boolean): LunaSampler {
  return { wrapU: texture.clamp ? 'clamp' : 'repeat', wrapV: texture.clamp ? 'clamp' : 'repeat', filter: texture.point ? 'point' : 'bilinear', srgb: !texture.linear && !gamma, mipmaps: false }
}

let dot: LunaTextureSpec | undefined

/** Unity's Default-Particle texture (a built-in, fileID 10300): a soft round dot, as the Unity-style player draws it. */
export function defaultParticleDot(): LunaTextureSpec {
  if (dot) return dot
  const size = 64
  const rgba = new Uint8ClampedArray(size * size * 4)
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const d = Math.hypot((x + 0.5) / size - 0.5, (y + 0.5) / size - 0.5)
      const t = Math.min(1, Math.max(0, (d - 0.15) / 0.35))
      rgba.set([255, 255, 255, Math.round(255 * (1 - t * t * (3 - 2 * t)))], (y * size + x) * 4)
    }
  }
  dot = { width: size, height: size, rgba, wrapU: 'clamp', wrapV: 'clamp', filter: 'bilinear', srgb: false, mipmaps: false }
  return dot
}

/** The texture of one slot: the decoded one, Unity's own dot when the slot points at it, else null (white). */
function slotTexture(input: MaterialInput, slot: string): LunaTextureSpec | null {
  const given = input.textures[slot]
  if (given) return given
  const ref = input.props.refs[slot]
  if (ref?.guid && (BUILTIN_SHADER_GUIDS as readonly string[]).includes(ref.guid) && ref.fileID === '10300') return defaultParticleDot()
  return null
}

const stOf = (props: MaterialProps, slot: string): Rgba => props.st[slot] ?? IDENTITY_ST

// ---- materials --------------------------------------------------------------------------------------------------

const spec = (shader: string | null, shaderName: string, parts: Partial<LunaMaterialSpec> = {}): LunaMaterialSpec => ({
  shader,
  shaderName,
  keywords: [],
  floats: {},
  colors: {},
  vectors: {},
  textures: {},
  notes: [],
  ...parts
})

/** What the host draws for a material that has none: Unity's Default-Particle (a soft dot, alpha blended). */
export function defaultParticleMaterial(): LunaMaterialSpec {
  return spec('default-particle', 'Default-Particle', {
    colors: { _TintColor: [0.5, 0.5, 0.5, 0.5] },
    vectors: { _MainTex_ST: IDENTITY_ST },
    textures: { _MainTex: defaultParticleDot() }
  })
}

/**
 * The texture slots `mapMaterial` reads for a shader, so the caller decodes only those. Undefined when the shader has no
 * port: the caller then names the colour map it draws (the one the Unity-style player uses) as the only slot.
 */
export function textureSlots(shaderName: string, props: MaterialProps): string[] | undefined {
  const id = BUILTIN_SHADERS[shaderName]
  if (id === 'urp-particles-unlit') return ['_BaseMap']
  if (id) return ['_MainTex']
  if (ALLIN1_SHADER.test(shaderName)) return ALLIN1_TEXTURES.filter((slot) => props.refs[slot])
  if (KLAUS_SHADER.test(shaderName)) return ['_Main_Tex']
  return undefined
}

/** Maps a parsed material. With no material at all, the result is Unity's Default-Particle. */
export function mapMaterial(input: MaterialInput | undefined): LunaMaterialSpec {
  if (!input) return defaultParticleMaterial()
  const name = input.shaderName ?? input.shader?.label ?? ''
  const id = BUILTIN_SHADERS[name]
  if (id === 'urp-particles-unlit') return mapUrpUnlit(input, name)
  if (id) return mapLegacy(input, id, name)
  if (ALLIN1_SHADER.test(name)) return mapAllIn1(input, name)
  if (KLAUS_SHADER.test(name)) return mapKlaus(input, name)
  return mapUnknown(input, name)
}

/** Legacy and mobile particle shaders: one colour map (`_MainTex`), and for the two with a tint, `_TintColor`. */
function mapLegacy(input: MaterialInput, id: string, name: string): LunaMaterialSpec {
  const colors = { ...input.defaults?.colors, ...input.props.colors }
  const tint = colors._TintColor
  return spec(id, name, {
    colors: { _TintColor: tint ? unityToRgba(tint) : [0.5, 0.5, 0.5, 0.5] },
    vectors: { _MainTex_ST: stOf(input.props, '_MainTex') },
    textures: { _MainTex: slotTexture(input, '_MainTex') }
  })
}

type BlendClass = 'alpha' | 'premultiply' | 'additive' | 'multiply'
const BLEND_INDEX: Record<BlendClass, number> = { alpha: 0, premultiply: 1, additive: 2, multiply: 3 }
const BLEND_BY_INDEX: readonly BlendClass[] = ['alpha', 'premultiply', 'additive', 'multiply']
/** The factors the URP inspector writes for each blend mode. */
const BLEND_FACTORS: Record<BlendClass, [number, number]> = {
  alpha: [SRC_ALPHA, ONE_MINUS_SRC_ALPHA],
  premultiply: [ONE, ONE_MINUS_SRC_ALPHA],
  additive: [SRC_ALPHA, ONE],
  multiply: [DST_COLOR, ZERO]
}

/** The blend a pair of Unity factors stands for, or undefined when it is none of the four. */
function classifyFactors(src: number | undefined, dst: number | undefined): BlendClass | undefined {
  if (src === undefined || dst === undefined) return undefined
  return (Object.keys(BLEND_FACTORS) as BlendClass[]).find((k) => BLEND_FACTORS[k][0] === src && BLEND_FACTORS[k][1] === dst)
}

const cullOf = (value: number | undefined): LunaShaderState['cull'] => (value === 0 || value === 1 ? value : 2)

/**
 * URP Particles/Unlit. The surface type, blend mode, colour mode and alpha clip are the numbers URP keeps on the
 * material. The blend is read from the factors that are drawn (`_SrcBlend`, `_DstBlend`), else `_Blend`, else the
 * keywords, else the look the parser worked out.
 */
function mapUrpUnlit(input: MaterialInput, name: string): LunaMaterialSpec {
  const { props, look } = input
  const floats = { ...input.defaults?.floats, ...props.floats }
  const colors = { ...input.defaults?.colors, ...props.colors }
  const on = (keyword: string) => props.keywords.includes(keyword)
  const notes: string[] = []

  const transparent = (floats._Surface ?? (look?.surface === 'transparent' ? 1 : 0)) >= 0.5
  const fromKeywords: BlendClass | undefined = on('_ALPHAPREMULTIPLY_ON') ? 'premultiply' : on('_ALPHAMODULATE_ON') ? 'multiply' : on('_ALPHABLEND_ON') ? 'alpha' : undefined
  const blend: BlendClass =
    classifyFactors(floats._SrcBlend, floats._DstBlend) ?? (floats._Blend !== undefined ? BLEND_BY_INDEX[Math.round(floats._Blend)] : undefined) ?? fromKeywords ?? look?.blend ?? 'alpha'

  const alphaClip = (floats._AlphaClip ?? (on('_ALPHATEST_ON') ? 1 : 0)) >= 0.5
  const colorMode = floats._ColorMode ?? (on('_COLOROVERLAY_ON') ? 3 : on('_COLORCOLOR_ON') ? 4 : on('_COLORADDSUBDIFF_ON') ? 1 : 0)
  if (colorMode === 3 || colorMode === 4) notes.push(`colour mode ${colorMode === 3 ? 'overlay' : 'colour'} is drawn as multiply`)

  const [src, dst] = transparent ? [floats._SrcBlend ?? BLEND_FACTORS[blend][0], floats._DstBlend ?? BLEND_FACTORS[blend][1]] : [ONE, ZERO]
  const state: LunaShaderState = {
    src,
    dst,
    zWrite: (floats._ZWrite ?? (transparent ? 0 : 1)) >= 0.5,
    cull: cullOf(floats._Cull),
    queue: transparent ? 3000 : alphaClip ? 2450 : 2000
  }
  const base = colors._BaseColor ?? colors._Color
  return spec('urp-particles-unlit', name, {
    state,
    keywords: [],
    floats: { _Surface: transparent ? 1 : 0, _Blend: BLEND_INDEX[blend], _ColorMode: colorMode, _AlphaClip: alphaClip ? 1 : 0, _Cutoff: floats._Cutoff ?? look?.cutoff ?? 0.5 },
    colors: { _BaseColor: base ? unityToRgba(base) : look?.color ? unityToRgba(look.color) : WHITE },
    vectors: { _BaseMap_ST: stOf(props, '_BaseMap') },
    textures: { _BaseMap: slotTexture(input, '_BaseMap') },
    notes
  })
}

const KEEP_STREAMS = 'custom vertex streams are not available in Luna, so the per-particle data of this shader stays at its start value'

/**
 * AllIn1Vfx: every float, colour, tiling and texture slot of the material, and its keywords. `_MainTex_TexelSize` is not
 * sent: the engine sets `<sampler>_TexelSize` itself for every sampler it binds, and a material value would clash with it.
 */
function mapAllIn1(input: MaterialInput, name: string): LunaMaterialSpec {
  const { props } = input
  const floats = { ...input.defaults?.floats, ...props.floats }
  const colors = { ...input.defaults?.colors, ...props.colors }
  const textures: Record<string, LunaTextureSpec | null> = {}
  const vectors: Record<string, Rgba> = {}
  for (const slot of ALLIN1_TEXTURES) {
    textures[slot] = slotTexture(input, slot)
    vectors[`${slot}_ST`] = stOf(props, slot)
  }
  return spec('all-in-1-vfx', name, {
    keywords: [...props.keywords],
    state: {
      src: floats._SrcMode ?? SRC_ALPHA,
      dst: floats._DstMode ?? ONE_MINUS_SRC_ALPHA,
      zWrite: floats._ZWrite === 1,
      cull: cullOf(floats._CullingOption ?? 0),
      queue: 3000
    },
    floats,
    colors: Object.fromEntries(Object.entries(colors).map(([k, c]) => [k, unityToRgba(c)])),
    vectors,
    textures,
    notes: [KEEP_STREAMS]
  })
}

/** VFX_Klaus: one formula in four shaders. The `_add` ones add, the `_apb` ones blend by alpha. */
function mapKlaus(input: MaterialInput, name: string): LunaMaterialSpec {
  const floats = { ...input.defaults?.floats, ...input.props.floats }
  const additive = /_add$/.test(name)
  return spec('klaus', name, {
    state: { src: SRC_ALPHA, dst: additive ? ONE : ONE_MINUS_SRC_ALPHA, zWrite: false, cull: 0, queue: 3000 },
    floats: { _Emission_Power: floats._Emission_Power ?? 1, uAdditive: additive ? 1 : 0 },
    vectors: { _Main_Tex_ST: stOf(input.props, '_Main_Tex') },
    textures: { _Main_Tex: slotTexture(input, '_Main_Tex') },
    notes: [KEEP_STREAMS]
  })
}

/**
 * A shader with no port. The host draws it with its fallback shader: the first colour map, times the material colour,
 * alpha blended, or added when the material says so. The host reports the shader name as `unsupported`.
 */
function mapUnknown(input: MaterialInput, name: string): LunaMaterialSpec {
  const { look } = input
  // A slot the caller decoded wins: it is the colour map the Unity-style player draws. Else the first colour slot the material fills.
  const slots = [...MAIN_SLOTS, ...Object.keys(input.textures)]
  const slot = slots.find((s) => input.textures[s]) ?? slots.find((s) => input.props.refs[s])
  const colour = look?.color ? unityToRgba(look.color) : WHITE
  const additive = look?.blend === 'additive'
  return spec(null, name || 'unknown shader', {
    // The fallback shader doubles its tint, so half of the colour gives the colour.
    colors: { _TintColor: [colour[0] / 2, colour[1] / 2, colour[2] / 2, colour[3] / 2] },
    vectors: { _MainTex_ST: slot ? stOf(input.props, slot) : IDENTITY_ST },
    textures: { _MainTex: slot ? slotTexture(input, slot) : null },
    state: { src: SRC_ALPHA, dst: additive ? ONE : ONE_MINUS_SRC_ALPHA, zWrite: false, cull: 0, queue: 3000 }
  })
}
