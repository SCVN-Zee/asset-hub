// The parse commands of the Rust side, typed. Each takes the file's place (`repo`, `branch`, `path`) and
// answers what a viewer draws; the bytes of the file do not cross the bridge for them. A failed call
// throws a `RemoteError` (`errors.<code>`), the same as `unwrap` does for the other calls.
//
// The commands answer untyped JSON; the shapes are in `@shared/types`. A number that is not finite
// serializes as `null` (a step tangent of a curve is infinite): `reviveTangents` puts `Infinity` back.

import { stash } from './api'
import { commands } from './bindings'
import { reviveTangents } from './tangents'
import { unwrap } from './remoteSource'
import type { IpcError, IpcResult } from '@shared/ipc'
import type {
  AnimClipInfo,
  AudioHeader,
  AudioImport,
  AudioImportSettings,
  Asset,
  BundleMaterial,
  DependencyReport,
  FactKind,
  FbxMetaInfo,
  FolderNode,
  FontImport,
  FontInfo,
  MaterialLook,
  MaterialProps,
  MaterialRef,
  ParticleEmitter,
  ParticleTotals,
  PrefabScene,
  ScriptCoverage,
  SpriteImport,
  Suggestion,
  TextureImport,
  TextureRole,
  TmpFontInfo
} from '@shared/types'

/** Where a file is; a `LiveContext` fits. */
export interface Place {
  repo: string
  branch: string
}

type Outcome = { status: 'ok'; data: unknown } | { status: 'error'; error: unknown }

/** The answer of a command, or a `RemoteError`; what the framework itself raises becomes one too. */
async function answer<T>(call: Promise<Outcome>): Promise<T> {
  let outcome: Outcome
  try {
    outcome = await call
  } catch (raised) {
    const message = typeof raised === 'string' ? raised : raised instanceof Error ? raised.message : String(raised)
    return unwrap<T>({ ok: false, error: { code: 'unknown', message } as IpcError })
  }
  const result: IpcResult<T> = outcome.status === 'ok' ? { ok: true, data: outcome.data as T } : { ok: false, error: outcome.error as IpcError }
  return unwrap(result)
}

/**
 * A value on its way to Rust: JSON has no Infinity (it would arrive as `null`, which a number field
 * refuses), so a step tangent is sent as the largest finite number. Rust only reads such values.
 */
function forRust<T>(value: T): T {
  return JSON.parse(JSON.stringify(value, (_key, v: unknown) => (v === Infinity ? Number.MAX_VALUE : v === -Infinity ? -Number.MAX_VALUE : v))) as T
}

// ---- shapes ------------------------------------------------------------------------------------

export interface MaterialData {
  look: MaterialLook
  props: MaterialProps
  /** Size of the .mat file. */
  bytes: number
}

export interface ShaderInfo {
  name: string | null
  defaults: Pick<MaterialProps, 'floats' | 'colors'>
  blend: MaterialLook['blend'] | null
  /** Texture slots the shader reads. */
  textures: string[]
}

export interface TextureInfo {
  kind: 'psd' | 'tiff' | null
  width: number | null
  height: number | null
  import: TextureImport | null
  sprite: SpriteImport | null
  role: TextureRole | null
  /** Larger side after the import max size; null when the size is not known. */
  builtSize: number | null
}

export interface ModelInfo {
  /** Mesh fileID and name pairs the .meta lists. */
  meshNames: Array<[string, string]>
  importScale: { globalScale: number; useFileScale: boolean }
  meta: FbxMetaInfo
}

export interface AudioInfo {
  header: AudioHeader
  import: AudioImport | null
  /** The settings a build takes, from the import. */
  effective: AudioImportSettings | null
}

export type FontData = Omit<FontInfo, 'codepoints'> & { codepointCount: number; /** "Aa", or what the font can show instead. */ sample: string; /** The code points it has glyphs for, as [first, last] runs. */ codepointRanges: Array<[number, number]>; coverage: ScriptCoverage[]; missingVietnamese: string; import: FontImport | null }

export type TmpFontData = Omit<TmpFontInfo, 'characters'> & { characterCount: number; coverage: ScriptCoverage[]; missingVietnamese: string }

/** A decoded mesh: attribute arrays ready for a `BufferGeometry`. */
export interface MeshData {
  positions: Float32Array
  normals?: Float32Array
  uvs?: Float32Array
  indices: Uint32Array
}

/** Pixels, RGBA, top row first. */
export interface Pixels {
  width: number
  height: number
  rgba: Uint8ClampedArray<ArrayBuffer>
}

export interface EmbeddedTexture extends Pixels {
  /** Colour data meant as linear, not sRGB. */
  linear: boolean
  clamp: boolean
  /** Point filtering. */
  point: boolean
}

export interface TmpAtlas {
  width: number
  height: number
  /** One coverage byte per pixel, bottom row first. */
  alpha: Uint8Array
}

/** What a handler declares; the rules for what its tab also lists and what lists leave out are data. */
export interface HandlerDescriptor {
  id: string
  labelKey: string
  badge: string
  previewKind: string
  materialsFromPrefabs: boolean
  usesGuids: boolean
  /** Name of what resolves its dependencies; null when it has none. */
  resolver: string | null
  facts: FactKind | null
  hidden: boolean
  delivery: 'unitypackage' | 'file'
  alsoShows: null | { kind: 'fact'; fromType: string; fact: FactKind } | { kind: 'animatedModel' }
  unlisted: null | 'notRigged'
}

export interface Browsed {
  roots: string[]
  root: string | null
  assets: Asset[]
  /** Assets the excluded folders hold back. */
  hidden: number
}

/** What a search found; `scores` holds `[asset id, score]` pairs in the order of `assets`. */
export interface SearchHit {
  assets: Asset[]
  related: string[]
  typos: string[]
  scores: Array<[string, number]>
}

// ---- decoding of the binary answers ------------------------------------------------------------

function view(buffer: ArrayBuffer): DataView {
  return new DataView(buffer)
}

/** `null` for the empty answer a command gives when the asset holds no such thing. */
export function decodeMesh(buffer: ArrayBuffer): MeshData | undefined {
  if (buffer.byteLength < 16) return undefined
  const v = view(buffer)
  const [version, vertices, indices, flags] = [v.getUint32(0, true), v.getUint32(4, true), v.getUint32(8, true), v.getUint32(12, true)]
  if (version !== 1) throw new Error(`The mesh has an unknown layout (${version})`)
  let at = 16
  const take = <A extends Float32Array | Uint32Array>(Type: { new (b: ArrayBuffer, o: number, n: number): A }, count: number): A => {
    const out = new Type(buffer.slice(at, at + count * 4), 0, count)
    at += count * 4
    return out
  }
  const positions = take(Float32Array, vertices * 3)
  const normals = flags & 1 ? take(Float32Array, vertices * 3) : undefined
  const uvs = flags & 2 ? take(Float32Array, vertices * 2) : undefined
  return { positions, normals, uvs, indices: take(Uint32Array, indices) }
}

export function decodePixels(buffer: ArrayBuffer): Pixels | undefined {
  if (buffer.byteLength < 8) return undefined
  const v = view(buffer)
  const [width, height] = [v.getUint32(0, true), v.getUint32(4, true)]
  return { width, height, rgba: new Uint8ClampedArray(buffer.slice(8, 8 + width * height * 4)) }
}

export function decodeEmbeddedTexture(buffer: ArrayBuffer): EmbeddedTexture | undefined {
  if (buffer.byteLength < 12) return undefined
  const v = view(buffer)
  const [width, height, flags] = [v.getUint32(0, true), v.getUint32(4, true), v.getUint32(8, true)]
  return { width, height, rgba: new Uint8ClampedArray(buffer.slice(12, 12 + width * height * 4)), linear: !!(flags & 1), clamp: !!(flags & 2), point: !!(flags & 4) }
}

export function decodeAtlas(buffer: ArrayBuffer): TmpAtlas | undefined {
  if (buffer.byteLength < 8) return undefined
  const v = view(buffer)
  const [width, height] = [v.getUint32(0, true), v.getUint32(4, true)]
  return { width, height, alpha: new Uint8Array(buffer.slice(8, 8 + width * height)) }
}

// ---- the commands ------------------------------------------------------------------------------

const bg = (background: boolean | undefined) => background ?? null

export const parse = {
  /** A mesh Unity saved as YAML; undefined when the asset holds none. */
  unityMesh: async (at: Place, path: string, background?: boolean) => decodeMesh(await answer<ArrayBuffer>(commands.unityMesh(at.repo, at.branch, path, bg(background)))),
  material: (at: Place, path: string, background?: boolean) => answer<MaterialData>(commands.material(at.repo, at.branch, path, bg(background))),
  shaderInfo: (at: Place, path: string, background?: boolean) => answer<ShaderInfo>(commands.shaderInfo(at.repo, at.branch, path, bg(background))),
  /** A texture saved inside another asset, by fileID. */
  embeddedTexture: async (at: Place, path: string, fileId: string, background?: boolean) => decodeEmbeddedTexture(await answer<ArrayBuffer>(commands.embeddedTexture(at.repo, at.branch, path, fileId, bg(background)))),
  /** A PSD or TIFF as pixels; undefined for any other file. */
  decodedImage: async (at: Place, path: string, background?: boolean) => decodePixels(await answer<ArrayBuffer>(commands.decodedImage(at.repo, at.branch, path, bg(background)))),
  /** `size` is the page's own reading of the picture, used when the file header gives none. */
  textureInfo: (at: Place, path: string, size?: { width: number; height: number }, background?: boolean) =>
    answer<TextureInfo>(commands.textureInfo(at.repo, at.branch, path, size?.width ?? null, size?.height ?? null, bg(background))),
  animClip: async (at: Place, path: string, background?: boolean) => reviveTangents(await answer<AnimClipInfo & { bytes: number }>(commands.animClip(at.repo, at.branch, path, bg(background)))),
  /** The scene of a prefab; Rust answers its nodes as `[id, node]` pairs, which become a `Map` here. */
  prefabScene: async (at: Place, path: string, background?: boolean): Promise<PrefabScene> => {
    const { nodes, ...rest } = await answer<Omit<PrefabScene, 'nodes'> & { nodes: Array<[string, PrefabScene['nodes'] extends Map<string, infer N> ? N : never]> }>(commands.prefabScene(at.repo, at.branch, path, bg(background)))
    return { ...rest, nodes: new Map(nodes) }
  },
  modelInfo: (at: Place, path: string, background?: boolean) => answer<ModelInfo>(commands.fbxInfo(at.repo, at.branch, path, bg(background))),
  /** Index of the mesh a particle system or prefab names by fileID among `names`; undefined when none. */
  meshForFileId: async (names: string[], fileId: string) => (await answer<number | null>(commands.meshForFileId(names, fileId))) ?? undefined,
  audioInfo: (at: Place, path: string, background?: boolean) => answer<AudioInfo>(commands.audioInfo(at.repo, at.branch, path, bg(background))),
  /** Bytes the clip takes in a build; undefined when the length is not known. */
  audioBuiltBytes: async (seconds: number, sampleRate: number, channels: number, settings?: AudioImport | null) =>
    (await answer<number | null>(commands.audioBuiltBytes(seconds, sampleRate, channels, settings ?? null))) ?? undefined,
  /** An Ogg Vorbis file as a WAV, for when `decodeAudioData` cannot read it. */
  oggToWav: async (ogg: Uint8Array) => new Uint8Array(await answer<ArrayBuffer>(commands.audioOggToWav(await stash(ogg)))),
  fontInfo: (at: Place, path: string, background?: boolean) => answer<FontData | null>(commands.fontInfo(at.repo, at.branch, path, bg(background))),
  tmpFont: (at: Place, path: string, background?: boolean) => answer<TmpFontData | null>(commands.tmpFont(at.repo, at.branch, path, bg(background))),
  tmpAtlas: async (at: Place, path: string, background?: boolean) => decodeAtlas(await answer<ArrayBuffer>(commands.tmpFontAtlas(at.repo, at.branch, path, bg(background)))),
  fbxTextureNames: (at: Place, path: string, background?: boolean) => answer<string[]>(commands.fbxTextureNames(at.repo, at.branch, path, bg(background))),
  modelBundleMaterials: (report: DependencyReport | undefined, prefabMaterial: MaterialRef | undefined, named: string[]) =>
    answer<BundleMaterial[]>(commands.modelBundleMaterialsFor(report ? forRust(report) : null, prefabMaterial ?? null, named)),
  prefabBundleMaterials: (report: DependencyReport) => answer<BundleMaterial[]>(commands.prefabBundleMaterialsFor(forRust(report))),
  particleTotals: (emitters: readonly ParticleEmitter[]) => answer<ParticleTotals>(commands.particleTotals(forRust(emitters))),
  handlerDescriptors: () => answer<HandlerDescriptor[]>(commands.handlerDescriptors()),
  browseAssets: (at: Place, root: string | undefined, excludedFolders: readonly string[]) => answer<Browsed>(commands.browseAssets(at.repo, at.branch, root ?? null, [...excludedFolders])),
  searchBuild: (assets: readonly Asset[], extraNames?: Record<string, string[]>) => answer<number>(commands.searchBuild(assets, extraNames ?? null)),
  searchQuery: (handle: number, query: string, folder?: string) => answer<SearchHit>(commands.searchQuery(handle, query, folder ?? null)),
  searchWords: (handle: number) => answer<Array<{ word: string; count: number }>>(commands.searchWords(handle)),
  searchSuggest: (handle: number, query: string) => answer<Suggestion[]>(commands.searchSuggest(handle, query)),
  searchFolders: (handle: number) => answer<FolderNode>(commands.searchFolders(handle))
}
