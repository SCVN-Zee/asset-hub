import { describe, expect, it } from 'vitest'
import type { MaterialLook, MaterialProps } from '@shared/types'
import { ALLIN1_TEXTURES } from '../viewer/shaderPorts/allIn1Vfx'
import { SHADER_MODULES } from './host/shaders'
import { defaultParticleDot, defaultParticleMaterial, mapMaterial, samplerFromEmbedded, samplerFromImport, textureSlots, type MaterialInput } from './materialMap'
import type { LunaTextureSpec } from './protocol'

const props = (parts: Partial<MaterialProps> = {}): MaterialProps => ({ keywords: [], floats: {}, colors: {}, st: {}, refs: {}, ...parts })
const SAMPLER = { wrapU: 'repeat', wrapV: 'repeat', filter: 'bilinear', srgb: true, mipmaps: true } as const
const texture = (width = 4, height = 4): LunaTextureSpec => ({ width, height, rgba: new Uint8ClampedArray(width * height * 4), ...SAMPLER })
const input = (label: string, parts: Partial<MaterialInput> = {}, kind: 'builtin' | 'package' | 'project' = 'builtin'): MaterialInput => ({
  shader: { label, kind },
  props: props(),
  textures: {},
  ...parts
})

describe('mapMaterial: legacy and mobile particle shaders', () => {
  const NAMES: Array<[string, string]> = [
    ['Legacy Shaders/Particles/Additive', 'legacy-additive'],
    ['Legacy Shaders/Particles/Additive (Soft)', 'legacy-additive-soft'],
    ['Legacy Shaders/Particles/Alpha Blended', 'legacy-alpha-blended'],
    ['Legacy Shaders/Particles/Alpha Blended Premultiply', 'legacy-alpha-blended-premultiply'],
    ['Legacy Shaders/Particles/Multiply', 'legacy-multiply'],
    ['Mobile/Particles/Additive', 'mobile-additive'],
    ['Mobile/Particles/Alpha Blended', 'mobile-alpha-blended']
  ]

  it.each(NAMES)('%s draws with module %s', (label, id) => {
    const tex = texture()
    const m = mapMaterial(input(label, { props: props({ st: { _MainTex: [2, 3, 0.5, 0.25] } }), textures: { _MainTex: tex } }))
    expect(m.shader).toBe(id)
    expect(m.shaderName).toBe(label)
    expect(m.textures._MainTex).toBe(tex)
    expect(m.vectors._MainTex_ST).toEqual([2, 3, 0.5, 0.25])
    expect(m.notes).toEqual([])
  })

  it('every module id that a Unity name maps to exists in the host, and the host module lists that name', () => {
    for (const [label, id] of NAMES) {
      expect(SHADER_MODULES[id], id).toBeDefined()
      expect(SHADER_MODULES[id].unityNames).toContain(label)
    }
  })

  it('reads _TintColor and falls back to the shader default of half grey', () => {
    const tinted = mapMaterial(input('Legacy Shaders/Particles/Additive', { props: props({ colors: { _TintColor: { r: 1, g: 0.5, b: 0.25, a: 0.75 } } }) }))
    expect(tinted.colors._TintColor).toEqual([1, 0.5, 0.25, 0.75])
    expect(mapMaterial(input('Legacy Shaders/Particles/Additive')).colors._TintColor).toEqual([0.5, 0.5, 0.5, 0.5])
  })

  it('prefers the name a project shader declares, and leaves an empty texture slot null', () => {
    const m = mapMaterial(input('whatever.shader', { shaderName: 'Mobile/Particles/Additive' }, 'project'))
    expect(m.shader).toBe('mobile-additive')
    expect(m.textures._MainTex).toBeNull()
  })

  it('uses the soft dot for a slot that points at Unity\'s Default-Particle texture', () => {
    const m = mapMaterial(input('Legacy Shaders/Particles/Alpha Blended', { props: props({ refs: { _MainTex: { fileID: '10300', guid: '0000000000000000f000000000000000' } } }) }))
    expect(m.textures._MainTex).toBe(defaultParticleDot())
  })
})

describe('mapMaterial: URP Particles/Unlit', () => {
  const URP = 'Universal Render Pipeline/Particles/Unlit'
  const map = (floats: Record<string, number>, rest: Partial<MaterialProps> = {}, look?: MaterialLook) => mapMaterial(input(URP, { props: props({ floats, ...rest }), look }))

  it('draws a transparent alpha material with the URP inspector state', () => {
    const m = map({ _Surface: 1, _Blend: 0, _SrcBlend: 5, _DstBlend: 10, _ZWrite: 0, _Cull: 2 })
    expect(m.shader).toBe('urp-particles-unlit')
    expect(m.state).toEqual({ src: 5, dst: 10, zWrite: false, cull: 2, queue: 3000 })
    expect(m.floats._Surface).toBe(1)
    expect(m.floats._Blend).toBe(0)
  })

  it.each([
    ['premultiply', 1, [1, 10]],
    ['additive', 2, [5, 1]],
    ['multiply', 3, [2, 0]]
  ] as const)('maps blend mode %s from _Blend alone', (_name, blend, factors) => {
    const m = map({ _Surface: 1, _Blend: blend })
    expect([m.state?.src, m.state?.dst]).toEqual(factors)
    expect(m.floats._Blend).toBe(blend)
  })

  it('takes the blend from the factors that are drawn when they disagree with _Blend', () => {
    const m = map({ _Surface: 1, _Blend: 0, _SrcBlend: 5, _DstBlend: 1 })
    expect(m.floats._Blend).toBe(2)
    expect([m.state?.src, m.state?.dst]).toEqual([5, 1])
  })

  it('falls back to the keywords, then to the parsed look', () => {
    expect(map({ _Surface: 1 }, { keywords: ['_ALPHAPREMULTIPLY_ON'] }).floats._Blend).toBe(1)
    expect(map({ _Surface: 1 }, { keywords: ['_ALPHAMODULATE_ON'] }).floats._Blend).toBe(3)
    expect(map({ _Surface: 1 }, {}, { surface: 'transparent', blend: 'additive' }).floats._Blend).toBe(2)
    expect(map({ _Surface: 1 }).floats._Blend).toBe(0)
  })

  it('draws an opaque material with One Zero, depth write and the geometry queue', () => {
    const m = map({ _Surface: 0, _SrcBlend: 5, _DstBlend: 10 })
    expect(m.state).toEqual({ src: 1, dst: 0, zWrite: true, cull: 2, queue: 2000 })
    expect(m.floats._Surface).toBe(0)
  })

  it('moves an alpha-clipped opaque material to the alpha-test queue and passes the cutoff', () => {
    const m = map({ _Surface: 0, _AlphaClip: 1, _Cutoff: 0.3 })
    expect(m.state?.queue).toBe(2450)
    expect(m.floats._AlphaClip).toBe(1)
    expect(m.floats._Cutoff).toBe(0.3)
    expect(map({ _Surface: 0 }, { keywords: ['_ALPHATEST_ON'] }).floats._AlphaClip).toBe(1)
  })

  it('takes the surface from the look when the material has no _Surface', () => {
    expect(map({}, {}, { surface: 'transparent' }).floats._Surface).toBe(1)
    expect(map({}, {}, { surface: 'opaque' }).floats._Surface).toBe(0)
  })

  it('reads cull and depth write from the material, and clamps a bad cull to back', () => {
    expect(map({ _Surface: 1, _Cull: 0, _ZWrite: 1 }).state).toMatchObject({ cull: 0, zWrite: true })
    expect(map({ _Surface: 1, _Cull: 7 }).state?.cull).toBe(2)
  })

  it('takes the base colour and base map, falling back to the look colour and white', () => {
    const tex = texture()
    const m = mapMaterial(input(URP, { props: props({ colors: { _BaseColor: { r: 0.1, g: 0.2, b: 0.3, a: 0.4 } }, st: { _BaseMap: [2, 2, 0, 0] } }), textures: { _BaseMap: tex } }))
    expect(m.colors._BaseColor).toEqual([0.1, 0.2, 0.3, 0.4])
    expect(m.textures._BaseMap).toBe(tex)
    expect(m.vectors._BaseMap_ST).toEqual([2, 2, 0, 0])
    expect(map({}, {}, { surface: 'transparent', color: { r: 1, g: 0, b: 0, a: 1 } }).colors._BaseColor).toEqual([1, 0, 0, 1])
    expect(map({}).colors._BaseColor).toEqual([1, 1, 1, 1])
  })

  it('keeps colour modes the shader does not draw as multiply and says so', () => {
    expect(map({ _ColorMode: 1 }).notes).toEqual([])
    expect(map({ _ColorMode: 3 }).notes).toHaveLength(1)
    expect(map({}, { keywords: ['_COLORCOLOR_ON'] }).floats._ColorMode).toBe(4)
  })
})

describe('mapMaterial: AllIn1Vfx', () => {
  const NAME = 'AllIn1Vfx/AllIn1Vfx'

  it('sends every texture slot with its tiling, the keywords and the draw state', () => {
    const tex = texture(8, 8)
    const m = mapMaterial(
      input('AllIn1Vfx', {
        shaderName: NAME,
        props: props({ keywords: ['GLOW_ON', 'FADE_ON'], floats: { _SrcMode: 5, _DstMode: 1, _ZWrite: 1, _CullingOption: 2, _Glow: 3 }, st: { _MainTex: [2, 2, 0, 0] } }),
        textures: { _MainTex: tex }
      }, 'project')
    )
    expect(m.shader).toBe('all-in-1-vfx')
    expect(m.keywords).toEqual(['GLOW_ON', 'FADE_ON'])
    expect(m.state).toEqual({ src: 5, dst: 1, zWrite: true, cull: 2, queue: 3000 })
    expect(m.floats._Glow).toBe(3)
    expect(Object.keys(m.textures).sort()).toEqual([...ALLIN1_TEXTURES].sort())
    expect(m.textures._MainTex).toBe(tex)
    expect(m.textures._FadeTex).toBeNull()
    expect(m.vectors._MainTex_ST).toEqual([2, 2, 0, 0])
    expect(m.vectors._FadeTex_ST).toEqual([1, 1, 0, 0])
    // The engine sets this one itself, and a material value clashes with it.
    expect(m.vectors._MainTex_TexelSize).toBeUndefined()
    expect(m.notes).toHaveLength(1)
  })

  it('lets the material override the shader defaults and defaults to alpha blending without depth write', () => {
    const m = mapMaterial(
      input('AllIn1Vfx', {
        shaderName: NAME,
        defaults: { floats: { _Alpha: 1, _Glow: 0.5 }, colors: { _Color: { r: 1, g: 1, b: 1, a: 1 } } },
        props: props({ floats: { _Glow: 2 } })
      }, 'project')
    )
    expect(m.floats).toMatchObject({ _Alpha: 1, _Glow: 2 })
    expect(m.colors._Color).toEqual([1, 1, 1, 1])
    expect(m.state).toEqual({ src: 5, dst: 10, zWrite: false, cull: 0, queue: 3000 })
  })
})

describe('mapMaterial: VFX_Klaus', () => {
  it.each([
    ['VFX_Klaus/fx_explosion_add', 1, 1],
    ['VFX_Klaus/Fx_Hit&Slash_add', 1, 1],
    ['VFX_Klaus/fx_explosion_apb', 10, 0],
    ['VFX_Klaus/Fx_Hit&Slash_apb', 10, 0]
  ])('%s', (name, dst, additive) => {
    const m = mapMaterial(input('Klaus', { shaderName: name, defaults: { floats: { _Emission_Power: 4 }, colors: {} }, props: props({ st: { _Main_Tex: [1, 2, 3, 4] } }) }, 'project'))
    expect(m.shader).toBe('klaus')
    expect(m.state?.dst).toBe(dst)
    expect(m.floats).toEqual({ _Emission_Power: 4, uAdditive: additive })
    expect(m.vectors._Main_Tex_ST).toEqual([1, 2, 3, 4])
    expect(Object.keys(m.textures)).toEqual(['_Main_Tex'])
  })
})

describe('mapMaterial: shaders without a port', () => {
  it('has no shader id, names the shader, and gives the fallback its colour map and half the colour', () => {
    const tex = texture()
    const m = mapMaterial(input('Custom/Glow', { props: props({ st: { _BaseColorMap: [2, 2, 0, 0] } }), look: { surface: 'transparent', color: { r: 1, g: 0.5, b: 0, a: 0.5 } }, textures: { _BaseColorMap: tex } }, 'project'))
    expect(m.shader).toBeNull()
    expect(m.shaderName).toBe('Custom/Glow')
    expect(m.textures._MainTex).toBe(tex)
    expect(m.vectors._MainTex_ST).toEqual([2, 2, 0, 0])
    expect(m.colors._TintColor).toEqual([0.5, 0.25, 0, 0.25])
    expect(m.state).toMatchObject({ src: 5, dst: 10 })
  })

  it('adds instead of blending when the material says additive', () => {
    expect(mapMaterial(input('Custom/Glow', { look: { surface: 'transparent', blend: 'additive' } }, 'project')).state).toMatchObject({ src: 5, dst: 1 })
  })

  it('still maps a material with no shader at all, and an unnamed one', () => {
    const m = mapMaterial({ shader: undefined, props: props(), textures: {} })
    expect(m.shader).toBeNull()
    expect(m.shaderName).toBe('unknown shader')
    expect(m.textures._MainTex).toBeNull()
  })

  it('does not treat Unity Standard Unlit particles as a ported shader', () => {
    expect(mapMaterial(input('Particles/Standard Unlit')).shader).toBeNull()
  })
})

describe('default particle', () => {
  it('maps a missing material to the Default-Particle with a soft dot', () => {
    const m = mapMaterial(undefined)
    expect(m).toEqual(defaultParticleMaterial())
    expect(m.shader).toBe('default-particle')
    expect(SHADER_MODULES[m.shader as string]).toBeDefined()
    const dot = defaultParticleDot()
    expect('rgba' in dot && dot.rgba.length).toBe(64 * 64 * 4)
    // Opaque in the middle, see-through at the corner.
    if ('rgba' in dot) {
      expect(dot.rgba[(32 * 64 + 32) * 4 + 3]).toBe(255)
      expect(dot.rgba[3]).toBe(0)
    }
  })
})

describe('texture sampling', () => {
  it('maps wrap modes: repeat (0 and the default -1), clamp, mirror and mirror once', () => {
    const wrap = (u: number | undefined, v: number | undefined) => {
      const s = samplerFromImport({ wrapU: u, wrapV: v, overrides: [] }, false)
      return [s.wrapU, s.wrapV]
    }
    expect(wrap(0, 0)).toEqual(['repeat', 'repeat'])
    expect(wrap(-1, undefined)).toEqual(['repeat', 'repeat'])
    expect(wrap(1, 2)).toEqual(['clamp', 'mirror'])
    expect(wrap(3, 1)).toEqual(['mirror', 'clamp'])
  })

  it('keeps mipmaps and sRGB as imported, and drops sRGB in a Gamma project', () => {
    expect(samplerFromImport({ sRGB: true, mipmaps: false, overrides: [] }, false)).toMatchObject({ srgb: true, mipmaps: false })
    expect(samplerFromImport({ sRGB: false, overrides: [] }, false).srgb).toBe(false)
    expect(samplerFromImport({ sRGB: true, overrides: [] }, true).srgb).toBe(false)
  })

  it('defaults to Unity\'s own import defaults when nothing is known', () => {
    expect(samplerFromImport(undefined, false)).toEqual({ wrapU: 'repeat', wrapV: 'repeat', filter: 'bilinear', srgb: true, mipmaps: true })
    expect(samplerFromImport(null, false, 'trilinear').filter).toBe('trilinear')
  })

  it('maps a texture saved inside another asset', () => {
    expect(samplerFromEmbedded({ linear: true, clamp: true, point: true }, false)).toEqual({ wrapU: 'clamp', wrapV: 'clamp', filter: 'point', srgb: false, mipmaps: false })
    expect(samplerFromEmbedded({ linear: false, clamp: false, point: false }, false)).toMatchObject({ wrapU: 'repeat', srgb: true })
    expect(samplerFromEmbedded({ linear: false, clamp: false, point: false }, true).srgb).toBe(false)
  })
})

describe('serialisable result', () => {
  it('survives a structured clone for every shader family (no functions or class instances)', () => {
    const inputs: MaterialInput[] = [
      input('Legacy Shaders/Particles/Additive'),
      input('Universal Render Pipeline/Particles/Unlit'),
      input('x', { shaderName: 'AllIn1Vfx/AllIn1Vfx' }, 'project'),
      input('x', { shaderName: 'VFX_Klaus/fx_explosion_add' }, 'project'),
      input('Custom/None', {}, 'project')
    ]
    for (const i of inputs) {
      const m = mapMaterial(i)
      expect(structuredClone(m)).toEqual(m)
    }
  })
})

describe('textureSlots', () => {
  it('names the one colour slot of a legacy, mobile or URP particle shader', () => {
    expect(textureSlots('Legacy Shaders/Particles/Additive', props())).toEqual(['_MainTex'])
    expect(textureSlots('Mobile/Particles/Alpha Blended', props())).toEqual(['_MainTex'])
    expect(textureSlots('Universal Render Pipeline/Particles/Unlit', props())).toEqual(['_BaseMap'])
  })

  it('names the AllIn1Vfx slots the material fills, and the Klaus slot', () => {
    const refs = { _MainTex: { fileID: '2800000', guid: 'a' }, _FadeTex: { fileID: '2800000', guid: 'b' }, _Other: { fileID: '2800000', guid: 'c' } }
    expect(textureSlots('AllIn1Vfx/AllIn1Vfx', props({ refs }))).toEqual(['_MainTex', '_FadeTex'])
    expect(textureSlots('VFX_Klaus/fx_explosion_add', props())).toEqual(['_Main_Tex'])
  })

  it('is undefined for a shader without a port, so the caller picks the colour map', () => {
    expect(textureSlots('Custom/Whatever', props())).toBeUndefined()
  })

  it('draws a shader without a port with the slot the caller decoded, not a stale one the material also fills', () => {
    const decoded = texture()
    const m = mapMaterial(
      input('Custom/Whatever', {
        props: props({ refs: { _BaseMap: { fileID: '2800000', guid: 'stale' }, _Foo: { fileID: '2800000', guid: 'real' } }, st: { _Foo: [2, 2, 0, 0] } }),
        textures: { _Foo: decoded }
      }, 'project')
    )
    expect(m.shader).toBeNull()
    expect(m.textures._MainTex).toBe(decoded)
    expect(m.vectors._MainTex_ST).toEqual([2, 2, 0, 0])
  })
})
