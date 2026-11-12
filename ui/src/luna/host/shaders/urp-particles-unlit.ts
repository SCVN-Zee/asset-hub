import { Blend, Queue, COLOR_MASK_ALL, particleFragment, particleVertex, type ParticleShaderDef, type ParticleShaderModule } from './common'

/**
 * Universal Render Pipeline/Particles/Unlit. Blend, depth write, cull and queue come from the material (`state`),
 * as the URP inspector writes them into `_SrcBlend`, `_DstBlend`, `_ZWrite` and `_Cull`. The code reads the same
 * material numbers URP does: `_Surface`, `_Blend` (0 alpha, 1 premultiply, 2 additive, 3 multiply), `_ColorMode`
 * (0 multiply, 1 additive, 2 subtract, 5 difference), `_AlphaClip` and `_Cutoff`. Colour modes overlay and colour are
 * drawn as multiply. Emission, soft particles and camera fading are left out.
 */
const FRAGMENT_UNIFORMS = `uniform 	mediump vec4 _BaseColor;
uniform 	float _Surface;
uniform 	float _Blend;
uniform 	float _ColorMode;
uniform 	float _AlphaClip;
uniform 	float _Cutoff;`

const BODY = `    mediump vec4 base = texel * _BaseColor;
    mediump vec4 color;
    if (_ColorMode > 0.5 && _ColorMode < 1.5) {
        color = vec4(base.rgb + vertexColor.rgb, base.a * vertexColor.a);
    } else if (_ColorMode > 1.5 && _ColorMode < 2.5) {
        color = vec4(base.rgb - vertexColor.rgb, base.a * vertexColor.a);
    } else if (_ColorMode > 4.5) {
        color = vec4(abs(base.rgb - vertexColor.rgb), base.a * vertexColor.a);
    } else {
        color = base * vertexColor;
    }
    if (_Surface < 0.5) {
        color.a = 1.0;
    }
    if (_AlphaClip > 0.5 && color.a - _Cutoff < 0.0) {
        discard;
    }
    if (_Surface > 0.5) {
        if (_Blend > 0.5 && _Blend < 1.5) {
            color.rgb *= color.a;
        } else if (_Blend > 2.5) {
            color.rgb = mix(vec3(1.0), color.rgb, color.a);
        }
    }
    SV_Target0 = color;`

/** What the URP inspector gives a transparent alpha-blended particle material. */
const DEFAULT_STATE = { src: Blend.SrcAlpha, dst: Blend.OneMinusSrcAlpha, zWrite: false, cull: 2 as const, queue: Queue.Transparent }

export const urpParticlesUnlit: ParticleShaderModule = {
  id: 'urp-particles-unlit',
  unityNames: ['Universal Render Pipeline/Particles/Unlit'],
  build: (options): ParticleShaderDef => {
    const state = options.state ?? DEFAULT_STATE
    return {
      name: 'Universal Render Pipeline/Particles/Unlit',
      vertex: particleVertex('_BaseMap_ST'),
      fragment: particleFragment(FRAGMENT_UNIFORMS, BODY, '_BaseMap'),
      blend: { src: state.src, dst: state.dst },
      zWrite: state.zWrite,
      cull: state.cull,
      colorMask: COLOR_MASK_ALL,
      queue: state.queue
    }
  }
}
