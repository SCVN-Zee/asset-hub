import { Blend, particleFragment, particleVertex, transparentState, type ParticleShaderModule } from './common'

/** Mobile/Particles/Additive: texture x vertex colour, SrcAlpha One. */
export const mobileAdditive: ParticleShaderModule = {
  id: 'mobile-additive',
  unityNames: ['Mobile/Particles/Additive'],
  build: () => ({
    name: 'Mobile/Particles/Additive',
    vertex: particleVertex('_MainTex_ST'),
    fragment: particleFragment('', '    SV_Target0 = vertexColor * texel;'),
    ...transparentState(Blend.SrcAlpha, Blend.One)
  })
}
