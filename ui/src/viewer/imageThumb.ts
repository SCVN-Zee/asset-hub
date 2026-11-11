import type { StatsSummary } from '@shared/types'
import type { LiveContext } from '../components/ModelViewer'
import { parse } from '../parse'
import { unwrap } from '../remoteSource'
import { builtSizeOf, decodeImage } from './textures'
import { MODEL_THUMB_VERSION } from './thumbVersions'

const W = 320
const H = 240

/**
 * Grid thumbnail of a texture: the image itself, fitted on a transparent background (the grid's stage shows behind it),
 * plus its measurements (size in pixels and bytes) for ranking.
 */
export async function renderImageThumbnail(live: LiveContext, path: string): Promise<{ url: string; stats: StatsSummary }> {
  const [bytes, info] = await Promise.all([window.api.assets.readFile(live.repo, live.branch, path, true).then(unwrap), parse.textureInfo(live, path, undefined, true)])
  const img = await decodeImage(live, path, bytes, info, info.import?.alphaSource, true)
  // Judged at the size it is built at: a 2048 px source capped to 512 on import is fine for playables.
  const built = await builtSizeOf(live, path, info, img, true)
  try {
    const canvas = document.createElement('canvas')
    canvas.width = W
    canvas.height = H
    const g = canvas.getContext('2d')!
    const scale = Math.min((W - 16) / img.width, (H - 16) / img.height, 1)
    const w = Math.max(1, Math.round(img.width * scale))
    const h = Math.max(1, Math.round(img.height * scale))
    g.drawImage(img.bitmap, (W - w) / 2, (H - h) / 2, w, h)
    return {
      url: canvas.toDataURL('image/webp', 0.85),
      stats: {
        triangles: 0,
        vertices: 0,
        bones: 0,
        clips: 0,
        materials: 0,
        fileBytes: 0,
        textureBytes: bytes.byteLength,
        maxTextureSize: built,
        modelThumb: MODEL_THUMB_VERSION,
        width: img.width,
        height: img.height,
        // A texture is measured in full from the file alone.
        source: 'viewer'
      }
    }
  } finally {
    img.bitmap.close()
  }
}
