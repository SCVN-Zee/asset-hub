import type { StatsSummary } from '@shared/types'
import { parse } from '../parse'
import type { LiveContext } from '../components/ModelViewer'
import { unwrap } from '../remoteSource'
import { offscreenHost } from './backgroundThumb'
import { MODEL_THUMB_VERSION } from './thumbVersions'
import { FbxViewer } from './FbxViewer'
import { fetchMaterialTextures, materialSlots, materialStats } from './materialPreview'

/**
 * Grid thumbnail of a material: a sphere with its textures and colour, plus
 * what its textures weigh. Uses what the branch already knows about GUIDs
 * nearby (`quick`), so a texture still unknown is left off until it is opened.
 */
export async function renderMaterialThumbnail(live: LiveContext, path: string, elsewhere = false): Promise<{ url: string; stats: StatsSummary }> {
  const [data, report] = await Promise.all([
    parse.material(live, path, true),
    // On another branch or repo: what is known already, not that branch's whole index (see backgroundThumb).
    window.api.assets.resolve(live.repo, live.branch, path, live.root, elsewhere ? 'instant' : 'full', true).then(unwrap)
  ])
  const material = report.materials[0]
  const textures = await fetchMaterialTextures(live, material, true)
  const viewer = new FbxViewer(offscreenHost())
  try {
    const { look } = data
    // Effects (transparent) on a flat card, like the viewer.
    viewer.showShape(look.surface === 'transparent' ? 'plane' : 'sphere')
    await viewer.applyMaterialSlots(materialSlots(material, textures))
    viewer.setLook(look)
    return { url: viewer.snapshot(), stats: { ...materialStats(material, textures, data.bytes, !report.partial), modelThumb: MODEL_THUMB_VERSION } }
  } finally {
    viewer.dispose()
    textures.forEach((x) => URL.revokeObjectURL(x.file.url))
  }
}
