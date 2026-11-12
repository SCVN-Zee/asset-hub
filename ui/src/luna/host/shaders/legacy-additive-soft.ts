import { Blend, particleFragment, particleVertex, transparentState, type ParticleShaderModule } from './common'

/**
 * Legacy Shaders/Particles/Additive (Soft): texture x vertex colour, One OneMinusSrcColor. There is no tint. The
 * soft-particle fade needs the scene depth, which this preview does not draw, so the fade is left out.
 */
export const legacyAdditiveSoft: ParticleShaderModule = {
  id: 'legacy-additive-soft',
  unityNames: ['Legacy Shaders/Particles/Additive (Soft)'],
  build: () => ({
    name: 'Legacy Shaders/Particles/Additive (Soft)',
    vertex: particleVertex('_MainTex_ST'),
    fragment: particleFragment('', '    SV_Target0 = vertexColor * texel;'),
    ...transparentState(Blend.One, Blend.OneMinusSrcColor)
  })
}
