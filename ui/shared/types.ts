// Domain types the page shares with the Rust commands. The parse commands answer untyped JSON; these are the shapes
// that JSON has (golden-checked against the 1.x parsers). Keep this file free of DOM and Node imports.

// Shared domain types. Keep this file free of DOM and Node imports so
// it can be used from the renderer and from tests.

/** One file or folder from a branch, shaped like GitHub's Git Trees API entries. */
export interface TreeEntry {
  path: string
  type: 'blob' | 'tree'
  /** Blob SHA from git. */
  sha: string
  /** Size in bytes as stored in git (for LFS files: the pointer size unless `lfsSize` is set). */
  size?: number
  /** True when the blob is a Git LFS pointer. */
  lfs?: boolean
  /** Real object size from the LFS pointer, when known. */
  lfsSize?: number
  /** sha256 of the real object from the LFS pointer — a content hash usable for dedupe. */
  lfsOid?: string
}

/** An entry that a handler recognised as an asset of its type. */
export interface Asset {
  /** Stable id within one repo + branch: the file path. */
  id: string
  /** Handler id, e.g. 'fbx'. */
  type: string
  path: string
  name: string
  sha: string
  size?: number
  lfs: boolean
}

/** Per-type statistics, stored as JSON in the index (`stats_json`). */
export type AssetStats = Record<string, number | string | boolean>

/** Kinds of assets known only by reading them (a scan per repo, kept by SHA). */
export type FactKind = 'effects' | 'sprites' | 'tmpFonts'

/** What is known about assets beyond their measurements, for tabs that list other types: blob SHAs per kind. */
export type AssetFacts = Partial<Record<FactKind, ReadonlySet<string>>>

export interface RepoRef {
  /** "owner/name" */
  fullName: string
  defaultBranch: string
}

export type ShaderKind = 'builtin' | 'package' | 'project'

export interface ShaderRef {
  guid: string
  label: string
  kind: ShaderKind
  path?: string
}

export interface TextureRef {
  /** Material slot, e.g. _BaseMap; absent for names found inside the FBX. */
  prop?: string
  guid?: string
  /** Repo path when found. */
  path?: string
  name: string
  /** How it was found: by GUID (exact) or by file name (heuristic, needs confirmation). */
  matchedBy: 'guid' | 'name'
  missing: boolean
  /** Not found yet, but the project was not fully searched: may still turn up. */
  unsure?: boolean
  size?: number
}

export interface MaterialRef {
  name: string
  /** Material name inside the FBX that this .mat replaces (externalObjects), used to place textures in the preview. */
  sourceName?: string
  guid?: string
  path?: string
  missing: boolean
  /** Not found yet, but the project was not fully searched: may still turn up. */
  unsure?: boolean
  shader?: ShaderRef
  textures: TextureRef[]
}

export type DependencyWarning =
  | 'missingMaterial'
  | 'missingTexture'
  | 'textureByName'
  | 'customShader'
  | 'noMeta'
  | 'noMaterials'
  | 'missingModel'
  | 'customScript'
  | 'particles'
  | 'uiPrefab'

/**
 * `known` only names prefabs read before that use the model (nothing loaded);
 * `cached` also loads their materials; `near` reads related folders; `all` reads every prefab.
 */
export type PrefabScope = 'known' | 'cached' | 'near' | 'all'

/** A prefab that uses a model, and the materials its renderers put on that model. */
export interface PrefabUsage {
  path: string
  materials: MaterialRef[]
}

export interface PrefabMaterialsResult {
  scope: PrefabScope
  prefabs: PrefabUsage[]
  /** Prefabs checked for this answer (scanned now or known from earlier scans). */
  scanned: number
  /** Prefabs not checked yet, and their size — what "scan all" would add. */
  remaining: number
  remainingBytes: number
  /** Materials, textures and shaders (with .meta) to add to an export. */
  files: string[]
  warnings: DependencyWarning[]
}

/** A model, nested prefab, script or other asset a prefab uses. */
export interface PartRef {
  kind: 'model' | 'prefab' | 'script' | 'other'
  guid: string
  name: string
  path?: string
  missing: boolean
  unsure?: boolean
  size?: number
  /** How many times the prefab (and the prefabs inside it) uses it. */
  uses: number
}

export interface DependencyReport {
  /** A: materials extracted to .mat; B: embedded in the FBX; none: material import disabled. */
  kind: 'A' | 'B' | 'none'
  materials: MaterialRef[]
  /** Textures referenced by name inside the FBX itself. */
  embeddedTextures: TextureRef[]
  /** Answered from the files nearest the model only; a full answer follows. */
  partial?: boolean
  warnings: DependencyWarning[]
  /**
   * .mat files next to an FBX that has no materials of its own — a guess, since
   * nothing references them. Not part of `files` unless the user picks one.
   */
  suggested: Array<{ material: MaterialRef; files: string[] }>
  /** Every repo path that an import would copy (assets and their .meta files). */
  files: string[]
  /** Prefabs: models, nested prefabs, scripts and other assets, in order of use. */
  parts?: PartRef[]
  /** Animation clips: models that may carry the rig it animates, most likely first. */
  rigs?: string[]
  /** Prefabs: GUID → repo path of everything found, so a preview can put the prefab together. */
  guidPaths?: Record<string, string>
  /** Prefabs: GUIDs of scripts not in the repo (from packages): the target project needs them too. */
  packageScripts?: string[]
  /** Prefabs: their particle systems, nested prefabs' included (`prefab` names the nested one). */
  particles?: Array<ParticleEmitter & { prefab?: string }>
}

/**
 * Format, sample rate and channels from an audio file's header. Web Audio
 * resamples what it decodes, so the source rate has to be read from the file.
 */
export interface AudioHeader {
  format: 'WAV' | 'OGG' | 'MP3' | 'AIFF' | string
  sampleRate?: number
  channels?: number
}

/** Model numbers measured in the viewer. */
export interface ModelStats {
  triangles: number
  vertices: number
  meshes: number
  materials: number
  bones: number
  animations: string[]
  textures: Array<{ name: string; width?: number; height?: number; bytes: number }>
  fileBytes: number
}

export interface BudgetIssue {
  key: 'triangles' | 'textureSize' | 'textureBytes' | 'fileBytes' | 'bones' | 'audioBytes' | 'audioLength' | 'fontBytes' | 'particles'
  value: number
  limit: number
}

/** Limits one asset is judged against for playable ads. */
export interface PlayableBudget {
  triangles: number
  /** Largest texture side, in pixels (as built). */
  textureSize: number
  textureBytes: number
  /** One model file. */
  fileBytes: number
  bones: number
  /** One audio clip as built: playables carry a few short sounds and one music loop. */
  audioBytes: number
  audioSeconds: number
  /** One font file: a whole font (Korean, CJK) easily outweighs the rest of a playable. */
  fontBytes: number
  /** Particles alive at once in one effect (all its emitters): each is drawn and simulated every frame. */
  particles: number
}

export type BudgetPreset = 'off' | 'standard' | 'tight' | 'custom'

/**
 * What is kept per model (by content SHA) so search results can be ranked
 * without opening them. Measured when a thumbnail renders or the model opens.
 */
export interface StatsSummary {
  triangles: number
  vertices: number
  bones: number
  clips: number
  materials: number
  fileBytes: number
  textureBytes: number
  /** Largest texture side in pixels (for a texture asset: as built, after the import max size). */
  maxTextureSize: number
  /** For a texture asset: its size in pixels (no triangles). */
  width?: number
  height?: number
  /** For an audio clip: its length in seconds (no triangles); `fileBytes` is then its estimated size in a build. */
  duration?: number
  /** Names of the animation clips inside a model (FBX), so they can be searched. */
  clipNames?: string[]
  /** For an animation clip: its length in seconds (no triangles). */
  clipLength?: number
  /** For a material: how many textures it uses (no triangles). */
  textureCount?: number
  /** For a font: how many glyphs it has; `fileBytes` is then the font file. */
  glyphs?: number
  /** For an effect (a prefab with particle systems): most particles alive at once, and how many emitters. */
  particles?: number
  emitters?: number
  /** Version of the effect card drawn for it (a still plus frames played on hover); older ones are drawn again. */
  fxThumb?: number
  /** Version of the rig search an animation clip's card was drawn with; older ones are drawn again. */
  animThumb?: number
  /** Version of the way a model or prefab card was drawn; older ones are drawn again. */
  modelThumb?: number
  /** For an animation clip: whether its card found a model to pose it on. */
  rigged?: boolean
  /** `viewer` counts every texture the model uses; `thumb` the ones named inside the FBX, plus those of its .mat files when `matTextures`. */
  source: 'thumb' | 'viewer'
  /** A model thumbnail that also counted the textures of its .mat files (older ones did not, and are measured again once). */
  matTextures?: true
}

/** What a TrueType / OpenType font file says about itself. */
export interface FontInfo {
  family: string
  /** Regular, Bold Italic… */
  style: string
  fullName?: string
  version?: string
  /** TrueType outlines (glyf) or CFF (OpenType .otf). */
  outlines: 'truetype' | 'cff'
  glyphs: number
  /** Unicode code points that map to a glyph. */
  codepoints: ReadonlySet<number>
  /** A collection (.ttc): only the first font is read. */
  collection?: number
}

export type FontScript = 'latin' | 'vietnamese' | 'korean' | 'japanese' | 'chinese' | 'cyrillic' | 'thai'

export interface ScriptCoverage {
  script: FontScript
  have: number
  total: number
  /** Enough to set text in it. */
  full: boolean
}

/** Unity's import settings for a font (TrueTypeFontImporter). */
export interface FontImport {
  /** Dynamic: rendered from the font at run time (the font file is in the build when `includeFontData`). */
  mode: 'dynamic' | 'unicode' | 'ascii' | 'asciiUpper' | 'asciiLower' | 'custom'
  includeFontData: boolean
  fontSize?: number
  /** Fonts Unity falls back to for missing characters. */
  fallbacks: string[]
}

/** One place an asset lives: a path in a repo, on one or more branches. */
export interface CatalogLocation {
  repo: string
  path: string
  /** Default branch first, then most recently committed. */
  branches: string[]
  /** In a listing (`listCatalog`), only the first branches are sent: how many there are in all. */
  branchTotal?: number
}

/** A file found anywhere in the catalog; identical files (same blob SHA) are one entry. */
export interface CatalogAsset extends Asset {
  locations: CatalogLocation[]
  /** In a listing (`listCatalog`), only the first locations are sent: how many there are in all. */
  locationTotal?: number
  branchCount: number
  repoCount: number
}

/** Repos and branches present in a listing, most files first, for its filters. */
export interface CatalogFacets {
  repos: Array<[string, number]>
  branches: Array<[string, number]>
}

/** What the grid shows for a scope: the assets left by the repo / branch filters, and the filters' choices. */
export interface CatalogListing {
  assets: CatalogAsset[]
  facets: CatalogFacets
}

/**
 * Where repos, branches and file trees come from. Phase 1 uses the mock
 * source; Phase 2 adds a GitHub-backed implementation behind the same interface.
 */
export interface AssetSource {
  readonly id: string
  listRepos(): Promise<RepoRef[]>
  listBranches(repo: string): Promise<string[]>
  listTree(repo: string, branch: string): Promise<TreeEntry[]>
}

/** Error codes shared by every asset source; the UI maps each to an i18n key `errors.<code>`. */
export type SourceErrorCode =
  | 'unauthorized'
  | 'sso'
  | 'forbidden'
  | 'notFound'
  | 'rateLimited'
  | 'network'
  | 'truncated'
  | 'readOnly'
  | 'unknown'

export type BuiltinMesh = 'cube' | 'cylinder' | 'sphere' | 'capsule' | 'plane' | 'quad'

export type ParticleRenderMode = 'billboard' | 'stretched' | 'horizontal' | 'vertical' | 'mesh' | 'none'

/** Optional modules worth knowing about (others, like colour over lifetime, cost little). */
export type ParticleModule = 'noise' | 'collision' | 'trails' | 'lights' | 'subEmitters' | 'textureSheet' | 'velocity' | 'force' | 'externalForces'

export interface ParticleEmitter {
  /** fileID of the ParticleSystem. */
  id: string
  name: string
  /** Seconds one cycle lasts. */
  duration: number
  looping: boolean
  prewarm: boolean
  /** Upper ends of the start values. */
  lifetime: number
  speed: number
  size: number
  maxParticles: number
  /** Particles per second. */
  rate: number
  /** Particles from bursts in one cycle. */
  bursts: number
  /** Most particles alive at once, as far as the settings tell: rate × lifetime + bursts, capped at the maximum. */
  peak: number
  shape?: string
  renderMode: ParticleRenderMode
  /** Material GUIDs: the particles', and the trails' when trails are on. */
  material?: string
  trailMaterial?: string
  /** Mesh GUID when particles are meshes. */
  mesh?: string
  /** Which mesh of that model (its fileID: models often hold many). */
  meshFileID?: string
  /** One of Unity's own meshes (Quad, Cube…), which no repo holds. */
  builtinMesh?: BuiltinMesh
  modules: ParticleModule[]
  /** Texture sheet animation grid (columns × rows). */
  sheet?: [number, number]
  subEmitters: number
  /** Transform (fileID) of its GameObject, to place it in the prefab. */
  node?: string
  /** What a preview needs to play it. */
  sim: ParticleSim
}

/** What a whole effect adds up to. */
export interface ParticleTotals {
  emitters: number
  peak: number
  /** Longest one-shot cycle, in seconds (0 when every emitter loops). */
  duration: number
  looping: boolean
}

/** One key; `v`, `inS` and `outS` have one number per component (3 for positions, 4 for rotations, 1 for floats). */
export interface CurveKey {
  t: number
  v: number[]
  inS: number[]
  outS: number[]
}

export interface TransformCurve {
  /** Transform path from the animated root, "" for the root itself. */
  path: string
  kind: 'position' | 'rotation' | 'euler' | 'scale'
  keys: CurveKey[]
}

export interface FloatCurve {
  path: string
  attribute: string
  /** Component class the property belongs to (4 Transform, 224 RectTransform, 95 Animator…). */
  classID: number
  keys: CurveKey[]
}

export type ClipKind = 'generic' | 'humanoid' | 'legacy' | 'ui' | 'sprite' | 'property'

export interface AnimClipInfo {
  name: string
  /** Seconds. */
  length: number
  sampleRate: number
  loop: boolean
  legacy: boolean
  /** Compressed curves are not stored as keys: nothing to preview. */
  compressed: boolean
  kind: ClipKind
  transformCurves: TransformCurve[]
  floatCurves: FloatCurve[]
  spriteCurves: number
  /** Sprites a sprite animation switches between. */
  spriteGuids: string[]
  events: number
  /** Distinct objects animated. */
  paths: string[]
  keyframes: number
}

export interface TmpFontInfo {
  name: string
  family: string
  style: string
  pointSize?: number
  atlasWidth: number
  atlasHeight: number
  padding?: number
  /** Dynamic assets add glyphs at run time from the source font, which then ships too. */
  dynamic: boolean
  /** Unicode code points it has glyphs for. */
  characters: number[]
  /** GUID of the font file it was made from. */
  sourceFont?: string
  /** GUIDs of the font assets it falls back to. */
  fallbacks: string[]
}

export type Vec3 = [number, number, number]

export type Quat = [number, number, number, number]

/** A Transform (or RectTransform) and its GameObject. Ids are the Transform's fileID. */
export interface SceneNode {
  id: string
  name: string
  /** Parent Transform id; absent for the root. */
  parent?: string
  position: Vec3
  rotation: Quat
  scale: Vec3
  active: boolean
  /** Placeholder for a Transform inside a nested instance ("stripped"): stands for that instance. */
  instance?: string
}

/** A MeshFilter + MeshRenderer pair, or a SkinnedMeshRenderer. */
export interface SceneMesh {
  node: string
  mesh: { fileID: string; guid: string }
  /** Material GUIDs by submesh. */
  materials: string[]
  skinned: boolean
}

/** A nested prefab or a model dropped in as it is. */
export interface SceneInstance {
  id: string
  source: string
  /** Transform it hangs under in this prefab. */
  parent?: string
  /** Overrides on the instance's root transform. */
  position?: Vec3
  rotation?: Quat
  scale?: Vec3
  /** Material overrides (applied to every renderer of the instance). */
  materials: string[]
  /** GameObject name override of the root. */
  name?: string
}

export interface PrefabScene {
  nodes: Map<string, SceneNode>
  meshes: SceneMesh[]
  instances: SceneInstance[]
  /** MonoBehaviour script GUIDs (custom or from packages). */
  scripts: string[]
  particleSystems: number
  /** Materials of particle renderers (particles, then trails), and meshes they emit. */
  particleMaterials: string[]
  particleMeshes: string[]
  /** RectTransforms: a UI prefab. */
  uiElements: number
  /** Every GUID the prefab references, whatever it is. */
  guids: string[]
}

/** Unity's MinMaxCurve: a constant, a curve (× scalar), random between two curves, or between two constants. */
export interface MinMax {
  mode: 0 | 1 | 2 | 3
  scalar: number
  min: number
  curve?: CurveKey[]
  minCurve?: CurveKey[]
}

export interface ColorKey {
  t: number
  /** r, g, b, a in 0–1. */
  c: [number, number, number, number]
}

export interface Gradient {
  colors: Array<{ t: number; r: number; g: number; b: number }>
  alphas: Array<{ t: number; a: number }>
}

/** Unity's MinMaxGradient: a colour, a gradient, random between two colours or two gradients, or a random colour of a gradient. */
export interface MinMaxColor {
  mode: 0 | 1 | 2 | 3 | 4
  max: [number, number, number, number]
  min: [number, number, number, number]
  gradient?: Gradient
  minGradient?: Gradient
}

export interface ParticleShape {
  /** Unity's ParticleSystemShapeType. */
  type: number
  radius: number
  /** 0 emits from the edge only, 1 from the whole volume. */
  radiusThickness: number
  /** Cone half angle, in degrees. */
  angle: number
  /** Degrees of the circle used (360 = all). */
  arc: number
  /** Cone volume length. */
  length: number
  position: [number, number, number]
  /** Euler degrees. */
  rotation: [number, number, number]
  scale: [number, number, number]
  randomDirection: number
}

export interface ParticleSim {
  duration: number
  looping: boolean
  prewarm: boolean
  /** Particles keep moving in the world (1) or with the emitter (0). */
  worldSpace: boolean
  simulationSpeed: number
  startDelay: MinMax
  startLifetime: MinMax
  startSpeed: MinMax
  startSize: MinMax
  /** Radians. */
  startRotation: MinMax
  /** Start height and depth, when the system sizes particles per axis (`startSize` is the width): a long thin ray. */
  startSize3D?: [MinMax, MinMax]
  /** Radians around X and Y too, when the system rotates particles in 3D (Z is `startRotation`). */
  startRotation3D?: [MinMax, MinMax]
  startColor: MinMaxColor
  gravity: MinMax
  maxParticles: number
  rate: MinMax
  bursts: Array<{ time: number; count: MinMax; cycles: number; interval: number; probability: number }>
  shape?: ParticleShape
  /** Added to each particle's velocity (local axes unless `velocityWorld`). */
  velocity?: [MinMax, MinMax, MinMax]
  velocityWorld?: boolean
  force?: [MinMax, MinMax, MinMax]
  /** Limit velocity: speed above `limit` is reduced by `dampen` each frame. */
  limit?: { limit: MinMax; dampen: number }
  colorOverLifetime?: MinMaxColor
  sizeOverLifetime?: MinMax
  /** Height and depth over lifetime, when the module scales each axis apart (`sizeOverLifetime` is the width). */
  sizeOverLifetime3D?: [MinMax, MinMax]
  /** Radians per second. */
  rotationOverLifetime?: MinMax
  /** Radians per second around X and Y, when the module turns each axis apart. */
  rotationOverLifetime3D?: [MinMax, MinMax]
  sheet?: { tilesX: number; tilesY: number; singleRow: boolean; row: number; frame: MinMax; startFrame: MinMax; cycles: number }
  noise?: { strength: MinMax; frequency: number; scrollSpeed: number }
  /** Systems this one starts at its particles (ParticleSystem fileIDs); they emit only then. */
  subEmitters?: Array<{ id: string; type: SubEmitterType; /** Unity's inherit flags: 1 colour, 2 size, 4 rotation, 8 lifetime. */ inherit: number; probability: number }>
  trail?: ParticleTrail
  /** Renderer settings for stretched billboards. */
  lengthScale: number
  velocityScale: number
  /** The renderer's pivot: a stretched particle moves along its stretch by y lengths, sideways by x sizes. */
  pivot: [number, number, number]
  /** Largest size on screen, as a fraction of the view height. */
  maxScreenSize: number
  /** Drawing order among transparent renderers: the order in layer first, then the fudge (lower is drawn later, on top). */
  sortingOrder: number
  sortingFudge: number
  /** Which way billboards face: the camera, or the world's / the system's own axes (a ring lying on the ground). */
  alignment: 'view' | 'world' | 'local' | 'facing' | 'velocity'
  /** Custom Data module, vector mode: up to 4 curves over each particle's lifetime (Custom1, Custom2). */
  custom1?: MinMax[]
  custom2?: MinMax[]
  /** The renderer's vertex streams (ParticleSystemVertexStream numbers), in order; absent: Unity's default set. */
  streams?: number[]
}

export type SubEmitterType = 'birth' | 'collision' | 'death' | 'trigger' | 'manual'

/** Trails behind particles (Trail module, Particles mode). */
export interface ParticleTrail {
  /** Share of particles that get a trail. */
  ratio: number
  /** How long a trail point lasts, as a share of the particle's lifetime. */
  lifetime: MinMax
  minVertexDistance: number
  sizeAffectsWidth: boolean
  inheritParticleColor: boolean
  width: MinMax
  colorOverLifetime: MinMaxColor
  colorOverTrail: MinMaxColor
}

export interface FbxMetaInfo {
  /** Materials remapped to .mat assets (kind A). */
  externalMaterials: Array<{ name: string; guid: string }>
  /** 0 = None, 1 = Legacy, 2 = Import via MaterialDescription. */
  materialImportMode?: number
  /** 0 = external (legacy name search), 1 = embedded in the model. */
  materialLocation?: number
}

export interface MaterialInfo {
  name?: string
  shader?: { fileID: string; guid: string }
  /** Material Variant parent (Unity 2022+). */
  parentGuid?: string
  /** Texture slots that reference a texture (empty slots are skipped). */
  textures: Array<{ prop: string; guid: string }>
}

export interface Rgba {
  r: number
  g: number
  b: number
  a: number
}

/** How a material looks, from its colour and number properties — what a preview can reproduce. */
export interface MaterialLook {
  /** _BaseColor (URP) or _Color (built-in): multiplies the colour texture. */
  color?: Rgba
  /** Only when the material turns emission on. */
  emission?: Rgba
  surface: 'opaque' | 'cutout' | 'transparent'
  /** Alpha clip threshold for a cutout surface. */
  cutoff?: number
  /** 0–1 (_Smoothness in URP, _Glossiness in built-in). */
  smoothness?: number
  metallic?: number
  /** How a transparent surface mixes with what is behind it, when the material says (particles mostly). */
  blend?: 'alpha' | 'premultiply' | 'additive' | 'multiply'
}

/** Everything a material sets, for previews that reproduce one shader exactly (see renderer viewer/shaderPorts). */
export interface MaterialProps {
  /** Shader keywords the material turns on (`GLOW_ON`…). */
  keywords: string[]
  floats: Record<string, number>
  colors: Record<string, Rgba>
  /** Tiling and offset of each texture slot, [scale x, scale y, offset x, offset y]. */
  st: Record<string, [number, number, number, number]>
  /** What each filled slot points at: an asset's GUID and the object in it (2800000 a texture file, else e.g. a texture saved inside a .mat). */
  refs: Record<string, { fileID: string; guid?: string }>
}

/** A texture saved inside another asset (a gradient a tool wrote into a .mat), decoded to RGBA. */
export interface EmbeddedTexture {
  width: number
  height: number
  rgba: Uint8Array<ArrayBuffer>
  /** Colour data meant as linear (m_ColorSpace 0), not sRGB. */
  linear: boolean
  clamp: boolean
  /** Point filtering (no blending between pixels). */
  point: boolean
}

export type TextureRole = 'color' | 'normal' | 'emission'

/** Import settings of a texture, from its .meta (TextureImporter). */
export interface TextureImport {
  /** 0 Default, 1 Normal map, 2 Editor GUI, 4 Cookie, 6 Lightmap, 7 Cursor, 8 Sprite, 10 Single channel. */
  textureType?: number
  /** Largest size Unity imports at, for the default platform. */
  maxTextureSize?: number
  /** 0 None, 1 Normal quality, 2 High quality, 3 Low quality. */
  compression?: number
  sRGB?: boolean
  mipmaps?: boolean
  alphaIsTransparency?: boolean
  /** Where alpha comes from: the file's own channel, none (opaque), or made from the colour's grey level. */
  alphaSource?: 'none' | 'input' | 'gray'
  /** Wrap mode across and down: 0 Repeat (also -1, the default), 1 Clamp, 2 Mirror, 3 Mirror once. */
  wrapU?: number
  wrapV?: number
  /** Per-platform overrides that are switched on. */
  overrides: Array<{ target: string; maxTextureSize?: number; compression?: number }>
}

/** One set of audio import settings (the default, or a platform's override). */
export interface AudioImportSettings {
  /** 0 Decompress On Load, 1 Compressed In Memory, 2 Streaming. */
  loadType?: number
  /** 0 PCM, 1 Vorbis, 2 ADPCM, 3 MP3, 7 AAC… */
  compressionFormat?: number
  /** 0–1, for Vorbis / MP3. */
  quality?: number
  /** 0 Preserve, 1 Optimize, 2 Override (sampleRateOverride). */
  sampleRateSetting?: number
  sampleRateOverride?: number
}

/** Import settings of an audio clip, from its .meta (AudioImporter). */
export interface AudioImport extends AudioImportSettings {
  forceToMono?: boolean
  preload?: boolean
  loadInBackground?: boolean
  /** Per-platform overrides, by platform name. */
  overrides: Array<AudioImportSettings & { target: string }>
}

/** 9-slice border in pixels (Unity stores it as x = left, y = bottom, z = right, w = top). */
export interface SpriteBorder {
  left: number
  bottom: number
  right: number
  top: number
}

/** One sprite cut from a texture; the rect is in pixels from the bottom-left corner, as Unity stores it. */
export interface SpriteRect {
  name: string
  x: number
  y: number
  width: number
  height: number
  border?: SpriteBorder
}

/** Sprite settings of a texture imported as Sprite (2D and UI), from its .meta. */
export interface SpriteImport {
  /** 1 Single, 2 Multiple (a sheet), 3 Polygon. */
  mode: number
  pixelsPerUnit?: number
  /** Single mode: the whole texture's 9-slice border. */
  border?: SpriteBorder
  /** Multiple mode: the sprites cut from the sheet. */
  sprites: SpriteRect[]
  /** Legacy sprite packer tag. */
  packingTag?: string
}

/**
 * A material to rebuild outside Unity (Blender). `target` is the material it
 * goes on in the imported file: a name (the FBX's own, or a .mat GUID in a
 * prefab's GLB), `*` for every material, or `?` for materials the file left
 * untextured (textures an FBX names by file).
 */
export interface BundleMaterial {
  target: string
  /** Name to give it in Blender (the .mat's). */
  name?: string
  /** Repo path of the .mat: colours and numbers come from it. */
  mat?: string
  /** Shader name, to tell unlit shaders. */
  shader?: string
  textures: Array<{ prop: string; path: string }>
}

export interface FolderNode {
  name: string
  /** Full folder path; '' for the root. */
  path: string
  children: FolderNode[]
  /** Assets in this folder and all sub-folders. */
  assetCount: number
}

export type MatchKind = 'name' | 'related' | 'folder' | 'typo'

export interface SearchResult {
  /** Matching assets, best first. */
  assets: Asset[]
  /** Group members that produced matches, e.g. ["shark", "tuna"] for "fish". */
  related: string[]
  /** Name words matched despite a typo, e.g. "wolf" for "wlof". */
  typos: string[]
  /** How well each asset (by id) matched: the name itself scores highest, then group members, folders, typos. */
  scores: ReadonlyMap<string, number>
}

export interface Suggestion {
  /** Text to put in the search box. */
  text: string
  /** How many assets it finds. */
  count: number
  /** For a group: a few of the names it covers. */
  examples?: string[]
}
