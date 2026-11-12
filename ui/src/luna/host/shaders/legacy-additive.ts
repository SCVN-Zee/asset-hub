import { Blend, particleFragment, particleVertex, transparentState, type ParticleShaderModule } from './common'

/** Legacy Shaders/Particles/Additive: texture x vertex colour x tint x 2, SrcAlpha One. */
export const legacyAdditive: ParticleShaderModule = {
  id: 'legacy-additive',
  unityNames: ['Legacy Shaders/Particles/Additive'],
  build: () => ({
    name: 'Legacy Shaders/Particles/Additive',
    vertex: particleVertex('_MainTex_ST'),
    fragment: particleFragment('uniform 	mediump vec4 _TintColor;', '    SV_Target0 = vertexColor * texel * _TintColor * 2.0;'),
    ...transparentState(Blend.SrcAlpha, Blend.One)
  })
}
