import { allIn1Fragment } from '../../../viewer/shaderPorts/allIn1Vfx'
import { Blend, COLOR_MASK_ALL, Queue, esToLunaFragment, particleVertex, type ParticleShaderModule } from './common'

/**
 * AllIn1Vfx, ported from the three.js fragment in `viewer/shaderPorts/allIn1Vfx.ts`. The shader turns its features
 * on per material with keywords, so a shader is compiled for each keyword set. Blend, depth write and cull come from
 * the material (`_SrcMode`, `_DstMode`, `_ZWrite`, `_CullingOption`). Luna has no custom vertex streams, so the
 * per-particle offsets and seed that the shader reads from TEXCOORD0 and TEXCOORD1 stay at zero.
 */
export const allIn1Vfx: ParticleShaderModule = {
  id: 'all-in-1-vfx',
  unityNames: ['AllIn1Vfx/AllIn1Vfx', 'AllIn1Vfx/AllIn1VfxBuiltIn', 'AllIn1Vfx/AllIn1VfxGrabPass'],
  build: (options) => {
    const state = options.state
    return {
      name: 'AllIn1Vfx/AllIn1Vfx',
      vertex: particleVertex(null),
      fragment: esToLunaFragment(allIn1Fragment(options.keywords)),
      blend: { src: state?.src ?? Blend.SrcAlpha, dst: state?.dst ?? Blend.OneMinusSrcAlpha },
      zWrite: state?.zWrite ?? false,
      cull: state?.cull ?? 0,
      colorMask: COLOR_MASK_ALL,
      queue: state?.queue ?? Queue.Transparent
    }
  }
}
