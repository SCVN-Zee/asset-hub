import { describe, expect, it } from 'vitest'
import { esToLunaFragment } from './common'
import { FALLBACK_SHADER, SHADER_MODULES } from './index'

describe('shader modules', () => {
  const modules = Object.values(SHADER_MODULES)

  it('has unique ids that match their keys, and a fallback that is registered', () => {
    expect(new Set(modules.map((m) => m.id)).size).toBe(modules.length)
    for (const [key, m] of Object.entries(SHADER_MODULES)) expect(m.id).toBe(key)
    expect(SHADER_MODULES[FALLBACK_SHADER.id]).toBe(FALLBACK_SHADER)
  })

  it.each(Object.keys(SHADER_MODULES))('%s builds a shader in the form the Luna particle patch rewrites', (id) => {
    const def = SHADER_MODULES[id].build({ keywords: [] })
    expect(def.vertex.startsWith('#version 300 es')).toBe(true)
    expect(def.fragment.startsWith('#version 300 es')).toBe(true)
    // The particle patch rewrites a standard compiler vertex shader that has these names.
    for (const name of ['hlslcc_mtx4x4unity_ObjectToWorld', 'hlslcc_mtx4x4unity_MatrixVP', 'in_POSITION0', 'in_COLOR0', 'in_TEXCOORD0']) expect(def.vertex).toContain(name)
    expect(def.fragment).toContain('SV_Target0')
    // GLSL ES 1.00 words do not compile in a 3.00 shader.
    expect(def.fragment).not.toMatch(/\b(texture2D|gl_FragColor)\b|^varying /m)
    expect([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]).toContain(def.blend.src)
    expect([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]).toContain(def.blend.dst)
    expect([0, 1, 2]).toContain(def.cull)
    expect(def.queue).toBeGreaterThanOrEqual(2000)
  })

  it.each([
    ['legacy-additive', 5, 1],
    ['legacy-additive-soft', 1, 6],
    ['legacy-alpha-blended', 5, 10],
    ['legacy-alpha-blended-premultiply', 1, 10],
    ['legacy-multiply', 0, 3],
    ['mobile-additive', 5, 1],
    ['mobile-alpha-blended', 5, 10]
  ])('%s blends with Unity factors %i, %i, without depth write, culling or an opaque queue', (id, src, dst) => {
    const def = SHADER_MODULES[id].build({ keywords: [] })
    expect(def.blend).toEqual({ src, dst })
    expect(def.zWrite).toBe(false)
    expect(def.cull).toBe(0)
    expect(def.queue).toBe(3000)
  })

  it('takes the render state of the URP and AllIn1Vfx shaders from the material', () => {
    const state = { src: 1, dst: 0, zWrite: true, cull: 1 as const, queue: 2450 }
    for (const id of ['urp-particles-unlit', 'all-in-1-vfx']) {
      const def = SHADER_MODULES[id].build({ keywords: [], state })
      expect(def.blend).toEqual({ src: 1, dst: 0 })
      expect(def.zWrite).toBe(true)
      expect(def.cull).toBe(1)
      expect(def.queue).toBe(2450)
    }
  })

  it('compiles AllIn1Vfx per keyword set', () => {
    const plain = SHADER_MODULES['all-in-1-vfx'].build({ keywords: [] }).fragment
    const glow = SHADER_MODULES['all-in-1-vfx'].build({ keywords: ['GLOW_ON'] }).fragment
    expect(plain).not.toContain('#define GLOW_ON')
    expect(glow).toContain('#define GLOW_ON')
  })

  it('lets the fallback ask for additive blending', () => {
    expect(FALLBACK_SHADER.build({ keywords: [] }).blend).toEqual({ src: 5, dst: 10 })
    expect(FALLBACK_SHADER.build({ keywords: [], state: { src: 5, dst: 1, zWrite: false, cull: 0, queue: 3000 } }).blend).toEqual({ src: 5, dst: 1 })
  })
})

describe('esToLunaFragment', () => {
  it('rewrites the three.js words and keeps the body', () => {
    const out = esToLunaFragment('uniform float uTime;\nvarying vec4 vColor;\nvoid main() { gl_FragColor = texture2D(t, vec2(0.0)) * vColor;\n#include <colorspace_fragment>\n}')
    expect(out).toContain('SV_Target0 = texture(t, vec2(0.0)) * vColor;')
    expect(out).not.toContain('colorspace_fragment')
    expect(out).not.toMatch(/^varying /m)
    expect(out).toContain('#define uTime (_Time.y)')
  })
})
