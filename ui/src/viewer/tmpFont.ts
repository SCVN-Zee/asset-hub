import type { StatsSummary } from '@shared/types'
import { parse, type TmpFontData } from '../parse'
import type { LiveContext } from '../components/ModelViewer'
import { FLAT_THUMB_VERSION } from './thumbVersions'

/** A TextMeshPro font asset read from the repo, with its atlas drawn as an image. */
export interface LoadedTmpFont {
  info: TmpFontData
  /** PNG data URL of the atlas: glyphs in white on transparent; absent when the atlas is not inside the asset. */
  atlas?: string
}

/** Draws an Alpha8 atlas (bottom row first, as Unity stores it) the right way up. */
function atlasImage(width: number, height: number, alpha: Uint8Array): string {
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const img = new ImageData(width, height)
  for (let y = 0; y < height; y++) {
    const src = (height - 1 - y) * width
    for (let x = 0; x < width; x++) {
      const o = (y * width + x) * 4
      img.data[o] = img.data[o + 1] = img.data[o + 2] = 255
      img.data[o + 3] = alpha[src + x]
    }
  }
  canvas.getContext('2d')!.putImageData(img, 0, 0)
  return canvas.toDataURL('image/png')
}

export async function loadTmpFont(live: LiveContext, path: string, background = false): Promise<LoadedTmpFont> {
  const [info, atlas] = await Promise.all([parse.tmpFont(live, path, background), parse.tmpAtlas(live, path, background)])
  if (!info) throw new Error('Not a TextMeshPro font asset')
  return { info, atlas: atlas && atlasImage(atlas.width, atlas.height, atlas.alpha) }
}

/**
 * Judged by what it adds to a build: its atlas (one byte per pixel) as the
 * file, the atlas side as a texture.
 */
export function tmpFontStats(font: LoadedTmpFont): StatsSummary {
  const { info } = font
  return {
    triangles: 0,
    vertices: 0,
    bones: 0,
    clips: 0,
    materials: 0,
    fileBytes: info.atlasWidth * info.atlasHeight,
    textureBytes: 0,
    maxTextureSize: Math.max(info.atlasWidth, info.atlasHeight),
    glyphs: info.characterCount,
    source: 'viewer'
  }
}

/** Grid thumbnail: the atlas (white glyphs) on a transparent background, so the card's grey stage shows through. */
export async function renderTmpFontThumbnail(live: LiveContext, path: string): Promise<{ url: string; stats: StatsSummary }> {
  const font = await loadTmpFont(live, path, true)
  const canvas = document.createElement('canvas')
  canvas.width = 320
  canvas.height = 240
  const g = canvas.getContext('2d')!
  if (font.atlas) {
    const img = new Image()
    // Set before `src`: the atlas is drawn to a canvas that is read back, which a tainted image would forbid.
    img.crossOrigin = 'anonymous'
    img.src = font.atlas
    await img.decode()
    const k = Math.min(300 / img.width, 220 / img.height)
    g.drawImage(img, 160 - (img.width * k) / 2, 120 - (img.height * k) / 2, img.width * k, img.height * k)
  } else {
    g.fillStyle = '#1f1f20'
    g.font = '600 44px sans-serif'
    g.textAlign = 'center'
    g.textBaseline = 'middle'
    g.fillText('TMP', 160, 120)
  }
  return { url: canvas.toDataURL('image/webp', 0.85), stats: { ...tmpFontStats(font), modelThumb: FLAT_THUMB_VERSION } }
}
