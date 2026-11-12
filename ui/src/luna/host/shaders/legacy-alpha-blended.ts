import { Blend, particleFragment, particleVertex, transparentState, type ParticleShaderModule } from './common'

/** Legacy Shaders/Particles/Alpha Blended: texture x vertex colour x tint x 2, SrcAlpha OneMinusSrcAlpha. */
export const legacyAlphaBlended: ParticleShaderModule = {
  id: 'legacy-alpha-blended',
  unityNames: ['Legacy Shaders/Particles/Alpha Blended'],
  build: () => ({
    name: 'Legacy Shaders/Particles/Alpha Blended',
    vertex: particleVertex('_MainTex_ST'),
    fragment: particleFragment('uniform 	mediump vec4 _TintColor;', '    SV_Target0 = vertexColor * texel * _TintColor * 2.0;'),
    ...transparentState(Blend.SrcAlpha, Blend.OneMinusSrcAlpha)
  })
}
