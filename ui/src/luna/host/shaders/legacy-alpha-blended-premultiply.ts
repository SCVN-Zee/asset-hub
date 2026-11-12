import { Blend, particleFragment, particleVertex, transparentState, type ParticleShaderModule } from './common'

/**
 * Legacy Shaders/Particles/Alpha Blended Premultiply: One OneMinusSrcAlpha. The shader scales colour and alpha by the
 * particle's alpha once more, as the Unity-style player in `viewer/particlePlayer.ts` does, so a texture's opaque
 * black background barely covers what is behind it. There is no tint.
 */
export const legacyAlphaBlendedPremultiply: ParticleShaderModule = {
  id: 'legacy-alpha-blended-premultiply',
  unityNames: ['Legacy Shaders/Particles/Alpha Blended Premultiply'],
  build: () => ({
    name: 'Legacy Shaders/Particles/Alpha Blended Premultiply',
    vertex: particleVertex('_MainTex_ST'),
    fragment: particleFragment('', '    SV_Target0 = vertexColor * texel * vertexColor.a;'),
    ...transparentState(Blend.One, Blend.OneMinusSrcAlpha)
  })
}
