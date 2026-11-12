import { KLAUS_FRAGMENT } from '../../../viewer/shaderPorts/klaus'
import { Blend, COLOR_MASK_ALL, Queue, esToLunaFragment, particleVertex, type ParticleShaderModule } from './common'

/**
 * VFX_Klaus (explosion, hit and slash packs), ported from the three.js fragment in `viewer/shaderPorts/klaus.ts`. The
 * additive and the alpha-blended shader differ in the blend and in `uAdditive`, which the material sets. The texture
 * packs its channels (R brightness, G dissolve mask, B distortion) and Custom1 drives the dissolve over each
 * particle's life. Luna has no custom vertex streams, so the dissolve stays at its start value.
 */
export const klaus: ParticleShaderModule = {
  id: 'klaus',
  unityNames: ['VFX_Klaus/fx_explosion_add', 'VFX_Klaus/fx_explosion_apb', 'VFX_Klaus/Fx_Hit&Slash_add', 'VFX_Klaus/Fx_Hit&Slash_apb'],
  build: (options) => {
    const state = options.state
    return {
      name: 'VFX_Klaus',
      vertex: particleVertex(null),
      fragment: esToLunaFragment(KLAUS_FRAGMENT),
      blend: { src: state?.src ?? Blend.SrcAlpha, dst: state?.dst ?? Blend.OneMinusSrcAlpha },
      zWrite: false,
      cull: 0,
      colorMask: COLOR_MASK_ALL,
      queue: Queue.Transparent
    }
  }
}
