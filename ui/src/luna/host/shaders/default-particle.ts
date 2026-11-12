import { Blend, transparentState, type ParticleShaderModule } from './common'
import { legacyAlphaBlended } from './legacy-alpha-blended'

/**
 * Draws Unity's built-in Default-Particle material (a soft dot, alpha blended) and every material whose shader has no
 * port. It is the alpha-blended legacy particle shader under its own name, so the fallback is easy to tell apart in a
 * log. A material may ask for additive blending through its state, which keeps a glow from drawing as a solid card.
 */
export const defaultParticle: ParticleShaderModule = {
  id: 'default-particle',
  unityNames: ['Default-Particle'],
  build: (options) => {
    const additive = options.state?.dst === Blend.One
    return { ...legacyAlphaBlended.build(options), name: 'AssetHub/Default Particle', ...(additive ? transparentState(Blend.SrcAlpha, Blend.One) : {}) }
  }
}
