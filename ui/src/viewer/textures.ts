import type { DependencyReport, MaterialRef, TextureImport } from '@shared/types'
import { baseName } from '../format'
import { parse, type Pixels, type TextureInfo } from '../parse'
import { unwrap } from '../remoteSource'
import type { LiveContext } from '../components/ModelViewer'
import { TGALoader } from 'three/examples/jsm/loaders/TGALoader.js'
import type { MaterialSlot, TextureFile } from './FbxViewer'

import { DRAWABLE } from './formats'

export { DRAWABLE }
const MIME: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', bmp: 'image/bmp' }

/** Pixels the Rust side decoded (PSD, TIFF), as PNG for the browser. */
async function pixelsToPng(img: Pixels): Promise<Blob | undefined> {
  const canvas = document.createElement('canvas')
  canvas.width = img.width
  canvas.height = img.height
  canvas.getContext('2d')!.putImageData(new ImageData(img.rgba, img.width, img.height), 0, 0)
  return (await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/png'))) ?? undefined
}

type AlphaSource = TextureImport['alphaSource']

/**
 * A picture with the alpha Unity gives it on import: "From Gray Scale" makes
 * it from the mean of red, green and blue (effect sheets drawn white on
 * black), "None" makes it opaque. Anything else keeps the file's own alpha.
 */
export async function applyAlphaSource(bitmap: ImageBitmap, source: AlphaSource): Promise<ImageBitmap> {
  if (source !== 'gray' && source !== 'none') return bitmap
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height)
  const g = canvas.getContext('2d')!
  g.drawImage(bitmap, 0, 0)
  const pixels = g.getImageData(0, 0, bitmap.width, bitmap.height)
  const d = pixels.data
  for (let i = 0; i < d.length; i += 4) d[i + 3] = source === 'gray' ? Math.round((d[i] + d[i + 1] + d[i + 2]) / 3) : 255
  bitmap.close()
  return createImageBitmap(pixels)
}

/** Downloads one texture from the repo and serves it through a blob URL (revoke it when done), with the alpha Unity imports it with. */
export async function fetchTextureFile(live: LiveContext, path: string, background = false): Promise<TextureFile | undefined> {
  if (!DRAWABLE.test(path)) return undefined
  try {
    const [bytes, info] = await Promise.all([window.api.assets.readFile(live.repo, live.branch, path, background).then(unwrap), parse.textureInfo(live, path, undefined, background)])
    const source = info.import?.alphaSource
    // Judged by content: .psd files, and PSD files saved with a .png name, become PNG for the browser.
    let blob = info.kind ? await parse.decodedImage(live, path, background).then((pixels) => (pixels ? pixelsToPng(pixels) : undefined)) : new Blob([bytes as Uint8Array<ArrayBuffer>], { type: MIME[path.split('.').pop()!.toLowerCase()] ?? 'application/octet-stream' })
    if (blob && (source === 'gray' || source === 'none')) {
      const bitmap = await applyAlphaSource(await createImageBitmap(blob), source)
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height)
      canvas.getContext('2d')!.drawImage(bitmap, 0, 0)
      bitmap.close()
      blob = await canvas.convertToBlob({ type: 'image/png' })
    }
    if (!blob) return undefined
    return { name: baseName(path), url: URL.createObjectURL(blob), bytes: bytes.byteLength, ...(info.role ? { role: info.role } : {}), ...(info.width && info.height ? { width: info.width, height: info.height } : {}) }
  } catch {
    // A texture that fails to download just renders untextured.
    return undefined
  }
}

/** A texture a .mat puts in a slot of a material, by repo path (not downloaded yet). */
export interface SlotPath {
  material: string
  prop: string
  path: string
}

/** Texture slots of one .mat, applied to the material named `material` (`*` for all). */
export function materialSlotPaths(m: MaterialRef, material: string): SlotPath[] {
  return m.textures.filter((tex) => tex.path).map((tex) => ({ material, prop: tex.prop ?? '', path: tex.path! }))
}

/** Kind A slots go to the FBX material the .mat replaces (its name inside the FBX), not the .mat's own name. */
export function reportSlotPaths(report: DependencyReport | undefined): SlotPath[] {
  return report?.kind === 'A' ? report.materials.flatMap((m) => materialSlotPaths(m, m.sourceName ?? m.name)) : []
}

const ROLE_PROP = { color: '_BaseMap', normal: '_BumpMap', emission: '_EmissionMap' } as const

/**
 * Slots for textures an FBX names without saying which material slot they
 * belong to (the model loads untextured): the role the Rust side guessed from
 * the file name is applied to every material.
 */
export function guessedSlots(files: readonly TextureFile[]): MaterialSlot[] {
  const slots: MaterialSlot[] = []
  for (const file of files) {
    const role = file.role ?? (files.length === 1 ? 'color' : undefined)
    if (role) slots.push({ material: '*', prop: ROLE_PROP[role], file })
  }
  return slots
}

/** A decoded image ready to draw, with what the texture panel reports about it. */
export interface DecodedImage {
  bitmap: ImageBitmap
  width: number
  height: number
  /** Some pixel is not fully opaque. */
  hasAlpha: boolean
  format: string
}

/** Format by content (a PSD saved as .png is still a PSD, known from the texture info), else by magic number, else by extension. */
function formatOf(path: string, bytes: Uint8Array, info: TextureInfo): string {
  if (info.kind === 'psd') return 'PSD'
  if (info.kind === 'tiff') return 'TIFF'
  if (bytes[0] === 0x89 && bytes[1] === 0x50) return 'PNG'
  if (bytes[0] === 0xff && bytes[1] === 0xd8) return 'JPEG'
  return (path.split('.').pop() ?? '').toUpperCase()
}

/** Samples the image for transparency on a small copy (enough to tell, cheap for large textures). */
function sampleAlpha(bitmap: ImageBitmap): boolean {
  const w = Math.min(128, bitmap.width)
  const h = Math.min(128, bitmap.height)
  const canvas = new OffscreenCanvas(w, h)
  const g = canvas.getContext('2d')!
  g.drawImage(bitmap, 0, 0, w, h)
  const px = g.getImageData(0, 0, w, h).data
  for (let i = 3; i < px.length; i += 4) if (px[i] < 255) return true
  return false
}

/** Decodes PNG / JPEG / GIF / BMP / WebP through the browser, PSD and TIFF through the Rust decoders, TGA through three. */
export async function decodeImage(live: LiveContext, path: string, bytes: Uint8Array, info: TextureInfo, alphaSource?: AlphaSource, background = false): Promise<DecodedImage> {
  const format = formatOf(path, bytes, info)
  let bitmap: ImageBitmap
  if (format === 'PSD' || format === 'TIFF') {
    const pixels = await parse.decodedImage(live, path, background)
    if (!pixels) throw new Error(format === 'PSD' ? 'Unsupported PSD' : 'Unsupported TIFF (CMYK, floating point or JPEG inside)')
    bitmap = await createImageBitmap(new ImageData(pixels.rgba, pixels.width, pixels.height))
  } else if (format === 'TGA') {
    // parse() returns raw pixel data (typed as a texture in three's typings).
    const tga = new TGALoader().parse(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer) as unknown as {
      data: Uint8Array
      width: number
      height: number
      flipY?: boolean
    }
    const data = new Uint8ClampedArray(tga.data.buffer, tga.data.byteOffset, tga.width * tga.height * 4) as Uint8ClampedArray<ArrayBuffer>
    bitmap = await createImageBitmap(new ImageData(data, tga.width, tga.height), { imageOrientation: tga.flipY ? 'flipY' : 'from-image' })
  } else {
    if (!DRAWABLE.test(path) && format !== 'PNG' && format !== 'JPEG') throw new Error(`${format} cannot be previewed`)
    bitmap = await createImageBitmap(new Blob([bytes as Uint8Array<ArrayBuffer>]))
  }
  // Shown with the alpha Unity imports it with.
  bitmap = await applyAlphaSource(bitmap, alphaSource)
  return { bitmap, width: bitmap.width, height: bitmap.height, hasAlpha: sampleAlpha(bitmap), format }
}

/** Larger side after the import max size: from the texture info when it knew the size, else asked again with the decoded size. */
export async function builtSizeOf(live: LiveContext, path: string, info: TextureInfo, decoded: { width: number; height: number }, background = false): Promise<number> {
  if (info.builtSize !== null) return info.builtSize
  return (await parse.textureInfo(live, path, decoded, background)).builtSize ?? Math.max(decoded.width, decoded.height)
}
