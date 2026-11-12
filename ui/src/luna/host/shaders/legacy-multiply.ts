import { Blend, particleFragment, particleVertex, transparentState, type ParticleShaderModule } from './common'

/** Legacy Shaders/Particles/Multiply: Zero SrcColor, white where the particle is see-through. */
export const legacyMultiply: ParticleShaderModule = {
  id: 'legacy-multiply',
  unityNames: ['Legacy Shaders/Particles/Multiply'],
  build: () => ({
    name: 'Legacy Shaders/Particles/Multiply',
    vertex: particleVertex('_MainTex_ST'),
    fragment: particleFragment(
      '',
      `    mediump vec4 prev = vertexColor * texel;
    SV_Target0 = vec4(mix(vec3(1.0), prev.rgb, prev.a), prev.a);`
    ),
    ...transparentState(Blend.Zero, Blend.SrcColor)
  })
}
