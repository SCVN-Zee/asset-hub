import type { AnimClipInfo, StatsSummary } from '@shared/types'
import { parse } from '../parse'
import { sampleCurve } from './curves'
import type { LiveContext } from '../components/ModelViewer'
import { unwrap } from '../remoteSource'
import { pickRig, rigOrder, THUMB_POSE, toThreeClip } from './animPlayer'
import { offscreenHost } from './backgroundThumb'
import { FbxViewer } from './FbxViewer'
import { ANIM_THUMB_VERSION } from './thumbVersions'

const W = 320
const H = 320
const COLORS = ['#7fb2ff', '#ff5ca8', '#3ddc97', '#f5a524', '#b48cff', '#5ad1e6']

/** Clips without a rig to show them on: their curves, each scaled to its own range, and their kind. */
function curveCard(info: AnimClipInfo, label: string): string {
  const canvas = document.createElement('canvas')
  canvas.width = W
  canvas.height = H
  const g = canvas.getContext('2d')!
  const series = [
    ...info.transformCurves.flatMap((c) => c.keys[0].v.map((_, i) => ({ keys: c.keys, i }))),
    ...info.floatCurves.map((c) => ({ keys: c.keys, i: 0 }))
  ].filter((s) => s.keys.length > 1)
  const length = info.length || 1
  series.slice(0, COLORS.length).forEach((s, n) => {
    const values = Array.from({ length: 64 }, (_, k) => sampleCurve(s.keys, (k / 63) * length)[s.i] ?? 0)
    const min = Math.min(...values)
    const span = Math.max(...values) - min || 1
    g.strokeStyle = COLORS[n]
    g.lineWidth = 2
    g.beginPath()
    values.forEach((v, k) => {
      const x = 16 + (k / 63) * (W - 32)
      const y = 40 + (1 - (v - min) / span) * (H - 90)
      if (k) g.lineTo(x, y)
      else g.moveTo(x, y)
    })
    g.stroke()
  })
  g.fillStyle = '#f1f0ec'
  g.font = '600 18px sans-serif'
  g.fillText(label, 16, H - 18)
  return canvas.toDataURL('image/webp', 0.85)
}

const CARD_LABEL: Record<AnimClipInfo['kind'], string> = { generic: 'Generic', humanoid: 'Humanoid', legacy: 'Legacy', ui: 'UI', sprite: 'Sprite', property: 'Properties' }

/**
 * Grid thumbnail of a clip: the model it animates (picked like the viewer
 * does) posed at 40% of the clip; clips with no rig found get their curves.
 */
export async function renderAnimThumbnail(live: LiveContext, path: string): Promise<{ url: string; stats: StatsSummary }> {
  const [info, report] = await Promise.all([
    parse.animClip(live, path, true),
    // Rig candidates need no GUID lookups.
    window.api.assets.resolve(live.repo, live.branch, path, live.root, 'instant', true).then(unwrap)
  ])
  const stats: StatsSummary = { triangles: 0, vertices: 0, bones: 0, clips: 1, materials: 0, fileBytes: info.bytes, textureBytes: 0, maxTextureSize: 0, clipLength: info.length, source: 'viewer', animThumb: ANIM_THUMB_VERSION }
  const posable = (info.kind === 'generic' || info.kind === 'legacy') && !info.compressed
  const best = posable && report.rigs?.length ? await pickRig(live, info, rigOrder(live, path, report.rigs), { background: true, tries: 3 }) : undefined
  if (!best) return { url: curveCard(info, CARD_LABEL[info.kind]), stats: { ...stats, rigged: false } }
  const viewer = new FbxViewer(offscreenHost())
  try {
    viewer.showGroup(best.rig.root)
    viewer.playClip(toThreeClip(info, best.rig), false)
    viewer.poseAt(info.length * THUMB_POSE)
    return { url: viewer.snapshot(), stats: { ...stats, rigged: true } }
  } finally {
    viewer.dispose()
  }
}
