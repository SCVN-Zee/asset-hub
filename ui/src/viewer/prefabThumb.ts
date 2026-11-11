import * as THREE from 'three'
import type { StatsSummary } from '@shared/types'
import { summarizeStats } from '../playable'
import type { LiveContext } from '../components/ModelViewer'
import { unwrap } from '../remoteSource'
import { offscreenHost } from './backgroundThumb'
import { FbxViewer } from './FbxViewer'
import { buildPrefab, dressPrefab } from './prefabBuild'
import { FX_THUMB_VERSION, filmEffect, particleStats } from './particleThumb'
import { buildEffect } from './effectBuild'
import { MODEL_THUMB_VERSION } from './thumbVersions'

/** A card for prefabs with nothing 3D to draw (UI, effects, scripts only): their kind, written large. */
function labelCard(label: string): string {
  const canvas = document.createElement('canvas')
  canvas.width = 320
  canvas.height = 240
  const g = canvas.getContext('2d')!
  g.fillStyle = '#f1f0ec'
  g.font = '600 44px sans-serif'
  g.textAlign = 'center'
  g.textBaseline = 'middle'
  g.fillText(label, 160, 120)
  return canvas.toDataURL('image/webp', 0.85)
}

/**
 * Grid thumbnail of a prefab: its models put together with their materials.
 * Measured like a model; effects are filmed through their run (`motion`,
 * played on hover) with the busiest frame as the card, and measured by particles.
 *
 * Dependencies come from the branch's GUID index (`full`): it is built once for the branch (one
 * download of every .meta, about 10 s on a large repo, started when the branch opens) and then
 * answers every card at once. Nearby lookups (`quick`) took 20-30 s per prefab on a cold cache,
 * four at a time, and no card showed for over 45 s.
 */
export async function renderPrefabThumbnail(live: LiveContext, path: string, elsewhere = false): Promise<{ url: string; stats?: StatsSummary; motion?: string }> {
  // On another branch or repo: what is known already, not that branch's whole index (see backgroundThumb).
  const report = unwrap(await window.api.assets.resolve(live.repo, live.branch, path, live.root, elsewhere ? 'instant' : 'full', true))
  const build = await buildPrefab(live, path, report, true)
  try {
    const fx = report.particles?.length ? report : undefined
    if (!build.meshes && !build.anchors.length) return { url: labelCard(report.warnings.includes('uiPrefab') ? 'UI' : '—') }
    const viewer = new FbxViewer(offscreenHost())
    const effect = build.anchors.length ? await buildEffect(live, report, build.anchors, true).catch(() => undefined) : undefined
    try {
      viewer.showGroup(build.group, effect?.measuredExtent())
      await dressPrefab(viewer, build)
      const film = effect && filmEffect(viewer, effect)
      const url = film?.still ?? viewer.snapshot()
      if (!fx) return { url, stats: { ...summarizeStats(viewer.stats(build.modelBytes, build.textures), report.partial ? 'thumb' : 'viewer'), modelThumb: MODEL_THUMB_VERSION } }
      const stats = build.meshes ? await particleStats(fx, summarizeStats(viewer.stats(build.modelBytes, build.textures), report.partial ? 'thumb' : 'viewer')) : await particleStats(fx)
      return { url, motion: film?.motion, stats: { ...stats, maxTextureSize: Math.max(stats.maxTextureSize, effect?.maxTextureSize ?? 0), modelThumb: MODEL_THUMB_VERSION, ...(film ? { fxThumb: FX_THUMB_VERSION } : {}) } }
    } finally {
      effect?.dispose()
      viewer.dispose()
    }
  } finally {
    build.dispose()
  }
}

/**
 * What each emitter of an effect does over its first seconds (for scripts that track down
 * effects drawn differently from Unity): its material and shader, whether a ported shader
 * draws it, the most particles alive, and where they were.
 */
export async function inspectEffect(live: LiveContext, path: string, seconds = 3): Promise<Array<Record<string, unknown>>> {
  const report = unwrap(await window.api.assets.resolve(live.repo, live.branch, path, live.root, 'quick'))
  const build = await buildPrefab(live, path, report, true)
  try {
    const effect = await buildEffect(live, report, build.anchors, true)
    const rows = effect.emitters.map((e) => {
      const m = report.materials.find((x) => x.guid === e.emitter.material)
      return { name: e.emitter.name, mode: e.emitter.renderMode, material: m?.name ?? e.emitter.material, shader: m?.shader?.label, ported: !!(e as unknown as { look: { custom?: unknown } }).look.custom, delay: e.emitter.sim.startDelay, max: 0, firstAt: -1, size: 0, at: [0, 0, 0] as number[] }
    })
    effect.restart()
    for (let t = 0; t < seconds; t += 1 / 30) {
      effect.update(1 / 30)
      effect.emitters.forEach((e, i) => {
        const n = (e as unknown as { count: number }).count
        if (n > rows[i].max) {
          rows[i].max = n
          e.sample((p, size) => {
            rows[i].at = [p.x, p.y, p.z].map((v) => Math.round(v * 100) / 100)
            rows[i].size = Math.round(size * 100) / 100
          })
        }
        if (n && rows[i].firstAt < 0) rows[i].firstAt = Math.round(t * 100) / 100
      })
    }
    effect.dispose()
    return rows
  } finally {
    build.dispose()
  }
}

/**
 * An effect filmed at given times (seconds from its start), side by side: to set beside Unity's frames at the same
 * times. `center` and `radius` (Unity's axes) are how the camera was framed, for Unity to frame the same way.
 */
export async function filmEffectAt(live: LiveContext, path: string, times: readonly number[], only?: readonly string[], width = 320, height = 240, zoom = 1): Promise<{ url: string; center: number[]; radius: number; floor: number }> {
  const report = unwrap(await window.api.assets.resolve(live.repo, live.branch, path, live.root, 'quick'))
  const build = await buildPrefab(live, path, report, true)
  const viewer = new FbxViewer(offscreenHost())
  try {
    const effect = await buildEffect(live, report, build.anchors, true)
    // Some emitters alone (by name), framed on them: to tell which part of an effect draws differently.
    if (only?.length) for (const e of effect.emitters) e.mesh.visible = only.includes(e.emitter.name)
    const extent = effect.measuredExtent()
    // The box the viewer frames (see FbxViewer.frameModel), taken before it moves the model.
    const box = new THREE.Box3().setFromObject(build.group).union(extent)
    const c = box.getCenter(new THREE.Vector3())
    const radius = Math.max(box.getSize(new THREE.Vector3()).length() / 2, 1e-3)
    viewer.showGroup(build.group, extent)
    await dressPrefab(viewer, build)
    if (zoom !== 1) viewer.zoomHome(zoom)
    const sheet = document.createElement('canvas')
    sheet.width = width * times.length
    sheet.height = height
    const g = sheet.getContext('2d')!
    effect.restart()
    let now = 0
    times.forEach((t, i) => {
      effect.advance(t - now)
      now = t
      viewer.drawFrame(g, i * width, 0, width, height)
    })
    effect.dispose()
    // three.js has X mirrored from Unity.
    return { url: sheet.toDataURL('image/png'), center: [-c.x, c.y, c.z], radius: radius * zoom, floor: box.min.y }
  } finally {
    viewer.dispose()
    build.dispose()
  }
}
