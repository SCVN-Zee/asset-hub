import { describe, expect, it } from 'vitest'
import type { MaterialRef, TextureRef } from '@shared/types'
import { mainTexture } from './particleThumb'

const tex = (prop: string, path?: string): TextureRef => ({ prop, path, name: prop, matchedBy: 'guid', missing: !path })
const mat = (...textures: TextureRef[]): MaterialRef => ({ name: 'm', missing: false, textures })

describe('main texture of a particle material', () => {
  it('takes the main slot', () => {
    expect(mainTexture(mat(tex('_Noise', 'n.png'), tex('_MainTex', 'fire.png')))).toBe('fire.png')
  })

  it('draws no texture when the main slot holds one of Unity’s own, rather than a noise map', () => {
    expect(mainTexture(mat(tex('_MainTex'), tex('_Noise', 'n.png')))).toBeUndefined()
  })

  it('takes the slot the shader reads, not a stale one left by another shader', () => {
    const legacy = { ...mat(tex('_BaseMap', 'stale.psd'), tex('_MainTex', 'ring.png')), shader: { guid: 'g', label: 'Legacy Shaders/Particles/Additive', kind: 'builtin' as const } }
    expect(mainTexture(legacy)).toBe('ring.png')
    const urp = { ...mat(tex('_MainTex', 'stale.psd'), tex('_BaseMap', 'leaf.png')), shader: { guid: 'g', label: 'Universal Render Pipeline/Lit', kind: 'package' as const } }
    expect(mainTexture(urp)).toBe('leaf.png')
  })

  it('follows the slots the shader declares when it was read', () => {
    const m = mat(tex('_BaseMap', 'atlas.png'), tex('_MainTex', 'palette.png'), tex('_Noise', 'n.png'))
    expect(mainTexture(m, ['_Noise', '_MainTex'])).toBe('palette.png')
    // Nothing it declares is filled: the usual order (no shader known: _MainTex first).
    expect(mainTexture(m, ['_Ramp'])).toBe('palette.png')
  })

  it('falls back to another colour slot, never a helper map', () => {
    expect(mainTexture(mat(tex('_DissolveTex', 'd.png'), tex('_Gradient', 'g.png')))).toBe('g.png')
    expect(mainTexture(mat(tex('_NoiseTex', 'n.png')))).toBeUndefined()
  })
})
