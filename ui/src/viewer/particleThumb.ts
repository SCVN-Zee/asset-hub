import type { DependencyReport, MaterialRef, StatsSummary } from '@shared/types'
import { parse } from '../parse'
import type { FbxViewer } from './FbxViewer'
import type { EffectPlayer } from './particlePlayer'

/** Main colour map of a particle material, by the slot names Unity's particle shaders use. */
const MAIN_PROPS = ['_MainTex', '_BaseMap', '_BaseColorMap', '_Texture', '_TintTex']

/** Slots that shape or distort a particle rather than colour it: never its look. */
const HELPER_PROP = /noise|mask|bump|normal|dissol|dist|flow|^_vo/i

/**
 * The texture that gives a particle material its look. A main slot holding
 * one of Unity's own textures (default white) or one not found means no
 * texture, not the next slot: that is often a noise or mask map, opaque.
 */
export function mainTexture(m: MaterialRef | undefined, declared?: readonly string[]): string | undefined {
  const textures = m?.textures ?? []
  // The shader's own list, when it was read: its first colour slot the material fills.
  const read = declared?.filter((p) => !HELPER_PROP.test(p) && textures.some((t) => t.prop === p))
  if (read?.length) return textures.find((t) => t.prop === read[0])?.path
  // Materials keep slots of shaders they used before (a stale _BaseMap on a legacy particle material):
  // the slot the shader reads comes first — _MainTex for Unity's own and hand-written shaders, _BaseMap for URP and Shader Graph.
  const legacy = !m?.shader || m.shader.kind === 'builtin' || (m.shader.kind === 'project' && /\.shader$/i.test(m.shader.path ?? ''))
  const order = legacy ? MAIN_PROPS : ['_BaseMap', ...MAIN_PROPS.filter((p) => p !== '_BaseMap')]
  const main = order.map((p) => textures.find((t) => t.prop === p)).find((t) => t)
  if (main) return main.path
  return textures.find((t) => t.path && !HELPER_PROP.test(t.prop ?? ''))?.path
}

/** Particle numbers of an effect, merged into what was measured of its models (if any). */
export async function particleStats(report: DependencyReport, base?: StatsSummary): Promise<StatsSummary> {
  const totals = await parse.particleTotals(report.particles ?? [])
  const guids = new Set((report.particles ?? []).flatMap((e) => [e.material, e.trailMaterial].filter((g): g is string => !!g)))
  const textures = new Map<string, number>()
  for (const m of report.materials) if (m.guid && guids.has(m.guid)) for (const t of m.textures) if (t.path) textures.set(t.path, t.size ?? 0)
  const textureBytes = [...textures.values()].reduce((a, b) => a + b, 0)
  return {
    ...(base ?? { triangles: 0, vertices: 0, bones: 0, clips: 0, materials: guids.size, fileBytes: 0, textureBytes: 0, maxTextureSize: 0, source: report.partial ? 'thumb' : 'viewer' }),
    ...(base ? {} : { textureBytes }),
    particles: totals.peak,
    emitters: totals.emitters
  }
}

/** Frames of a moving effect card, laid out as a square sheet. */
export const MOTION_FRAMES = 16
const MOTION_COLS = 4
/** Bumped when effect cards are drawn differently: older ones are drawn again once. v3: blend from the shader, no noise maps as colour; v4: Unity's own meshes, framed per emitter; v5: premultiplied blending, soft default dots; v6: alpha from grey level as Unity imports it; v7: billboards facing the system's axes; v8: 3D rotation per particle, the right mesh of a model, the slot the shader reads; v9: sizes per axis; v10: alpha per mesh particle;; v11: masks kept in a colour channel; v12: AllIn1Vfx and VFX_Klaus shaders ported, custom data and vertex streams; v13: wrapped keyword lines, Unity .mesh meshes, ported shaders on mesh particles,; meshes counted in framing; v14: Gamma projects drawn in Gamma, frames at exact times. */
export const FX_THUMB_VERSION = 16

/** Cache key of an effect's moving card, next to its still one. */
export const motionKey = (key: string) => `${key}.fx`

/**
 * Plays an effect through and films it: a sheet of frames covering its whole
 * run (one cycle for loops), for the card to play on hover, and the busiest
 * frame as the still card.
 */
export function filmEffect(viewer: FbxViewer, effect: EffectPlayer, width = 320, height = 240): { still: string; motion: string } {
  const rows = Math.ceil(MOTION_FRAMES / MOTION_COLS)
  const sheet = document.createElement('canvas')
  sheet.width = width * MOTION_COLS
  sheet.height = height * rows
  const g = sheet.getContext('2d')!
  const { warm, length } = effect.span()
  effect.advance(warm)
  let busiest = 0
  let most = -1
  for (let i = 0; i < MOTION_FRAMES; i++) {
    effect.advance(length / MOTION_FRAMES)
    viewer.drawFrame(g, (i % MOTION_COLS) * width, Math.floor(i / MOTION_COLS) * height, width, height)
    if (effect.alive > most) [busiest, most] = [i, effect.alive]
  }
  const still = document.createElement('canvas')
  still.width = width
  still.height = height
  still.getContext('2d')!.drawImage(sheet, (busiest % MOTION_COLS) * width, Math.floor(busiest / MOTION_COLS) * height, width, height, 0, 0, width, height)
  return { still: still.toDataURL('image/webp', 0.85), motion: sheet.toDataURL('image/webp', 0.8) }
}
