import type { FontImport, StatsSummary } from '@shared/types'
import { parse, type FontData } from '../parse'
import type { LiveContext } from '../components/ModelViewer'
import { unwrap } from '../remoteSource'
import { FLAT_THUMB_VERSION } from './thumbVersions'

/** A font read from the repo, registered with the page so text can be set in it. */
export interface LoadedFont {
  info: FontData
  /** Unicode code points it has glyphs for (the Rust side sends them as runs). */
  codepoints: ReadonlySet<number>
  /** CSS family name it was registered under (unique per load). */
  css: string
  bytes: number
  import?: FontImport
  /** Removes the font from the page. */
  release(): void
}

let serial = 0

/** Reads a font (and its .meta) and makes it usable in CSS and canvas; call `release` when done. */
export async function loadFont(live: LiveContext, path: string, background = false): Promise<LoadedFont> {
  const [bytes, info] = await Promise.all([window.api.assets.readFile(live.repo, live.branch, path, background).then(unwrap), parse.fontInfo(live, path, background)])
  if (!info) throw new Error('Not a TrueType / OpenType font')
  const css = `assethub-font-${++serial}`
  const face = new FontFace(css, bytes.slice().buffer)
  await face.load()
  document.fonts.add(face)
  const codepoints = new Set<number>()
  for (const [first, last] of info.codepointRanges) for (let c = first; c <= last; c++) codepoints.add(c)
  return { info, codepoints, css, bytes: bytes.byteLength, import: info.import ?? undefined, release: () => document.fonts.delete(face) }
}

export function fontStats(font: Pick<LoadedFont, 'info' | 'bytes'>): StatsSummary {
  return { triangles: 0, vertices: 0, bones: 0, clips: 0, materials: 0, fileBytes: font.bytes, textureBytes: 0, maxTextureSize: 0, glyphs: font.info.glyphs, source: 'viewer' }
}

/** Grid thumbnail: "Aa" and the family name, set in the font itself, on a transparent background. */
export async function renderFontThumbnail(live: LiveContext, path: string): Promise<{ url: string; stats: StatsSummary }> {
  const font = await loadFont(live, path, true)
  try {
    const canvas = document.createElement('canvas')
    canvas.width = 320
    canvas.height = 240
    const g = canvas.getContext('2d')!
    // Transparent: the card's grey stage (#767676) shows through, so the ink is dark.
    g.fillStyle = '#1f1f20'
    g.textAlign = 'center'
    g.textBaseline = 'middle'
    g.font = `96px "${font.css}"`
    g.fillText(font.info.sample, 160, 104, 296)
    g.fillStyle = '#000000'
    g.font = `22px "${font.css}", sans-serif`
    g.fillText(font.info.family || path.slice(path.lastIndexOf('/') + 1), 160, 196, 296)
    return { url: canvas.toDataURL('image/webp', 0.85), stats: { ...fontStats(font), modelThumb: FLAT_THUMB_VERSION } }
  } finally {
    font.release()
  }
}
