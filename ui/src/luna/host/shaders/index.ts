import type { ParticleShaderModule } from './common'
import { allIn1Vfx } from './all-in-1-vfx'
import { defaultParticle } from './default-particle'
import { klaus } from './klaus'
import { legacyAdditive } from './legacy-additive'
import { legacyAdditiveSoft } from './legacy-additive-soft'
import { legacyAlphaBlended } from './legacy-alpha-blended'
import { legacyAlphaBlendedPremultiply } from './legacy-alpha-blended-premultiply'
import { legacyMultiply } from './legacy-multiply'
import { mobileAdditive } from './mobile-additive'
import { mobileAlphaBlended } from './mobile-alpha-blended'
import { urpParticlesUnlit } from './urp-particles-unlit'

/** Every shader the host can draw with, by id. */
export const SHADER_MODULES: Readonly<Record<string, ParticleShaderModule>> = Object.fromEntries(
  [
    legacyAdditive,
    legacyAdditiveSoft,
    legacyAlphaBlended,
    legacyAlphaBlendedPremultiply,
    legacyMultiply,
    mobileAdditive,
    mobileAlphaBlended,
    urpParticlesUnlit,
    klaus,
    allIn1Vfx,
    defaultParticle
  ].map((m) => [m.id, m])
)

/** Draws every material whose shader has no port. */
export const FALLBACK_SHADER = defaultParticle
