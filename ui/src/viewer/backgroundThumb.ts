import type { DependencyReport, StatsSummary } from '@shared/types'
import { parse } from '../parse'
import { summarizeStats } from '../playable'
import type { LiveContext } from '../components/ModelViewer'
import { unwrap } from '../remoteSource'
import { FbxViewer, type MaterialSlot, type TextureFile } from './FbxViewer'
import { fetchTextureFile, guessedSlots, reportSlotPaths } from './textures'
import { MODEL_THUMB_VERSION } from './thumbVersions'

/** Keeps background renders light: a few textures, the .mat ones first. Others still count by their size. */
const MAX_TEXTURES = 8

let host: HTMLDivElement | undefined

/** Off-screen element that background viewers render into. */
export function offscreenHost(): HTMLDivElement {
  if (!host) {
    host = document.createElement('div')
    host.setAttribute('aria-hidden', 'true')
    Object.assign(host.style, { position: 'fixed', left: '-10000px', top: '0', width: '320px', height: '240px', pointerEvents: 'none' })
    document.body.appendChild(host)
  }
  return host
}

/** How long a card waits for the branch's GUID index before it is drawn with what is known. */
const FAST_MS = 1200
/** Models drawn quickly this session (before the index was ready): their redraw waits for it. */
const drawnQuickly = new Set<string>()

/**
 * The model's materials for its card. The branch's GUID index (`full`) knows them all, but the first
 * time a branch is opened it takes several seconds to build, and every card waited for it (a search
 * showed no picture for 6 s). So a card waits a moment, then is drawn with what is already known
 * (`quick`: its shape and the textures the FBX names); drawn without its .mat textures, it is drawn
 * again once the grid is idle (Browse `remeasure`), and that time it waits for the index.
 */
async function materialsFor(live: LiveContext, path: string, elsewhere: boolean): Promise<{ report?: DependencyReport; quick: boolean }> {
  // A result on another branch or repo: what is known already (GUIDs are shared by a repo's branches),
  // never that branch's whole index; the card is drawn in full once that branch is opened.
  if (elsewhere) {
    const known = await window.api.assets.resolve(live.repo, live.branch, path, live.root, 'instant', true).then(unwrap, () => undefined)
    return { report: known, quick: !known || !!known.partial }
  }
  const key = `${live.repo}@${live.branch}:${path}`
  const full = window.api.assets.resolve(live.repo, live.branch, path, live.root, 'full', true).then(unwrap, () => undefined)
  if (drawnQuickly.has(key)) return { report: await full, quick: false }
  const first = await Promise.race([full.then((report) => ({ report })), new Promise<undefined>((done) => setTimeout(done, FAST_MS))])
  if (first) return { report: first.report, quick: false }
  const known = await window.api.assets.resolve(live.repo, live.branch, path, live.root, 'instant', true).then(unwrap, () => undefined)
  if (known && !known.partial) return { report: known, quick: false }
  drawnQuickly.add(key)
  return { report: known, quick: true }
}

/**
 * Renders a grid thumbnail without opening the model: downloads the FBX, the
 * textures of its .mat files (kind A, from GUIDs the branch already knows
 * nearby) and the textures it names, draws one frame and returns a data URL.
 * Every texture counts toward the size, even one not downloaded.
 */
export async function renderThumbnail(live: LiveContext, path: string, elsewhere = false): Promise<{ url: string; stats: StatsSummary }> {
  const [bytes, names, { report, quick }] = await Promise.all([window.api.assets.readFile(live.repo, live.branch, path, true).then(unwrap), parse.fbxTextureNames(live, path, true), materialsFor(live, path, elsewhere)])
  const found = names.length ? unwrap(await window.api.assets.findByNames(live.repo, live.branch, names, path, live.root)) : {}
  const slotPaths = reportSlotPaths(report)
  const all = [...new Set([...slotPaths.map((s) => s.path), ...Object.values(found).filter((p): p is string => !!p)])]
  const fetched = await Promise.all(all.slice(0, MAX_TEXTURES).map(async (p) => [p, await fetchTextureFile(live, p, true)] as const))
  const byPath = new Map(fetched.filter((e): e is readonly [string, TextureFile] => !!e[1]))
  const files = [...byPath.values()]
  const viewer = new FbxViewer(offscreenHost())
  try {
    await viewer.load(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer, new Map(files.map((f) => [f.name.toLowerCase(), f])))
    const slots = slotPaths.flatMap((s): MaterialSlot[] => {
      const file = byPath.get(s.path)
      return file ? [{ material: s.material, prop: s.prop, file }] : []
    })
    if (slots.length) await viewer.applyMaterialSlots(slots)
    else if (!viewer.hasColorMaps() && files.length) await viewer.applyMaterialSlots(guessedSlots(files))
    // Measured while the model is loaded anyway: lets results be ranked without opening them.
    // Drawn before its .mat textures were known: no `matTextures`, so the card is drawn again.
    const stats: StatsSummary = { ...summarizeStats(viewer.stats(bytes.byteLength, files), 'thumb'), ...(quick ? {} : { matTextures: true as const }), modelThumb: MODEL_THUMB_VERSION }
    stats.textureBytes += skippedBytes(report, new Set(byPath.keys()))
    return { url: viewer.snapshot(), stats }
  } finally {
    viewer.dispose()
    files.forEach((f) => URL.revokeObjectURL(f.url))
  }
}

/** Size of the .mat textures left out of the render (over the limit, or not drawable like EXR). */
function skippedBytes(report: DependencyReport | undefined, drawn: ReadonlySet<string>): number {
  const sizes = new Map<string, number>()
  for (const m of report?.kind === 'A' ? report.materials : []) for (const t of m.textures) if (t.path && t.size && !drawn.has(t.path)) sizes.set(t.path, t.size)
  return [...sizes.values()].reduce((a, b) => a + b, 0)
}
