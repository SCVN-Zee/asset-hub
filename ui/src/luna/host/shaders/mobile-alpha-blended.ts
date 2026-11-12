import { Blend, particleFragment, particleVertex, transparentState, type ParticleShaderModule } from './common'

/** Mobile/Particles/Alpha Blended: texture x vertex colour, SrcAlpha OneMinusSrcAlpha. */
export const mobileAlphaBlended: ParticleShaderModule = {
  id: 'mobile-alpha-blended',
  unityNames: ['Mobile/Particles/Alpha Blended'],
  build: () => ({
    name: 'Mobile/Particles/Alpha Blended',
    vertex: particleVertex('_MainTex_ST'),
    fragment: particleFragment('', '    SV_Target0 = vertexColor * texel;'),
    ...transparentState(Blend.SrcAlpha, Blend.OneMinusSrcAlpha)
  })
}
