/**
 * Messages between the app window and the Luna host iframe (`luna-host.html`).
 * Both sides import only these types, so the iframe stays free of app code and the app stays free of Luna globals.
 */

/** Pixels of a texture, sent as an ImageBitmap (preferred) or as raw RGBA bytes (top row first). */
export type LunaTexture = { bitmap: ImageBitmap } | { width: number; height: number; rgba: Uint8ClampedArray }

/** RGBA, each channel 0..1. */
export type Rgba = readonly [number, number, number, number]

/** How a texture is sampled, as Unity's import settings and filter mode set it. */
export interface LunaSampler {
  wrapU: 'repeat' | 'clamp' | 'mirror'
  wrapV: 'repeat' | 'clamp' | 'mirror'
  filter: 'point' | 'bilinear' | 'trilinear'
  /** Colour data stored as sRGB. Only matters when the host draws in linear space. */
  srgb: boolean
  mipmaps: boolean
}

export type LunaTextureSpec = LunaTexture & LunaSampler

/** Fixed-function state of a shader that Unity keeps outside the shader code. Values are Unity's enums. */
export interface LunaShaderState {
  /** `UnityEngine.Rendering.BlendMode` of the source and the destination. */
  src: number
  dst: number
  zWrite: boolean
  /** `UnityEngine.Rendering.CullMode`: 0 off, 1 front, 2 back. */
  cull: 0 | 1 | 2
  /** Render queue: 2000 geometry, 2450 alpha test, 3000 transparent. */
  queue: number
}

/** One material. The host draws it with the shader module named by `shader`, or with a fallback when there is none. */
export interface LunaMaterialSpec {
  /** Id of a host shader module, or null when no module draws `shaderName`. */
  shader: string | null
  /** The Unity shader the material came from. Reported when the host falls back. */
  shaderName: string
  /** Unity keywords the material turns on. Only shaders that compile per keyword read them. */
  keywords: string[]
  /** State for shaders that take it from the material (URP, AllIn1Vfx). Other modules fix their own. */
  state?: LunaShaderState
  floats: Record<string, number>
  colors: Record<string, Rgba>
  /** Vectors by property name, such as `_MainTex_ST` (scale x, scale y, offset x, offset y). */
  vectors: Record<string, Rgba>
  /** Textures by property name. Null draws the slot white. */
  textures: Record<string, LunaTextureSpec | null>
  /** Things the preview could not reproduce for this material. The host reports each as `unsupported`. */
  notes: string[]
}

export type Vec3 = readonly [number, number, number]

/** One key of an animation curve. A step is an infinite tangent. */
export interface LunaKey {
  t: number
  v: number
  inTangent: number
  outTangent: number
}

/** Unity's MinMaxCurve. */
export interface LunaCurve {
  /** 0 constant, 1 curve, 2 two curves, 3 two constants. */
  mode: 0 | 1 | 2 | 3
  /** The constant (mode 0), the upper constant (mode 3), or the multiplier of the curves (modes 1 and 2). */
  scalar: number
  /** The lower constant (mode 3). */
  min: number
  curve?: LunaKey[]
  minCurve?: LunaKey[]
}

export interface LunaGradient {
  colors: Array<{ t: number; r: number; g: number; b: number }>
  alphas: Array<{ t: number; a: number }>
  /** Steps instead of a blend. */
  fixed: boolean
}

/** Unity's MinMaxGradient. */
export interface LunaColor {
  /** 0 colour, 1 gradient, 2 two colours, 3 two gradients, 4 random colour of a gradient. */
  mode: 0 | 1 | 2 | 3 | 4
  max: Rgba
  min: Rgba
  gradient?: LunaGradient
  minGradient?: LunaGradient
}

export interface LunaBurst {
  time: number
  /** The count is a whole number between these, both ends included. */
  min: number
  max: number
  cycles: number
  interval: number
}

export interface LunaMain {
  duration: number
  loop: boolean
  prewarm: boolean
  startDelay: LunaCurve
  startLifetime: LunaCurve
  startSpeed: LunaCurve
  /** One curve for all axes, or width, height and depth. */
  startSize: LunaCurve | [LunaCurve, LunaCurve, LunaCurve]
  /** Radians around Z, or around X, Y and Z. */
  startRotation: LunaCurve | [LunaCurve, LunaCurve, LunaCurve]
  startColor: LunaColor
  gravity: LunaCurve
  /** 0 local, 1 world. */
  simulationSpace: 0 | 1
  simulationSpeed: number
  useUnscaledTime: boolean
  /** 0 hierarchy, 1 local, 2 shape. */
  scalingMode: number
  maxParticles: number
  /** 0 transform, 1 rigidbody. */
  emitterVelocityMode: number
  playOnAwake: boolean
}

export interface LunaEmission {
  rateOverTime: LunaCurve
  rateOverDistance: LunaCurve
  bursts: LunaBurst[]
}

export interface LunaShape {
  /** Unity's ParticleSystemShapeType. Only the values Luna implements get here. */
  type: number
  angle: number
  radius: number
  radiusThickness: number
  /** 0 random, 1 loop, 2 ping-pong. */
  radiusMode: number
  radiusSpread: number
  length: number
  arc: number
  arcMode: number
  arcSpread: number
  arcSpeed: LunaCurve
  donutRadius: number
  position: Vec3
  /** Euler degrees. */
  rotation: Vec3
  scale: Vec3
  randomDirection: number
  sphericalDirection: number
  randomPosition: number
  alignToDirection: boolean
}

export interface LunaVelocity {
  x: LunaCurve
  y: LunaCurve
  z: LunaCurve
  world: boolean
  orbital: [LunaCurve, LunaCurve, LunaCurve]
  orbitalOffset: [LunaCurve, LunaCurve, LunaCurve]
  radial: LunaCurve
  speedModifier: LunaCurve
}

export interface LunaLimit {
  limit: LunaCurve
  dampen: number
  drag: LunaCurve
  dragBySize: boolean
  dragByVelocity: boolean
  world: boolean
}

export interface LunaForce {
  x: LunaCurve
  y: LunaCurve
  z: LunaCurve
  world: boolean
  randomized: boolean
}

export interface LunaSizeCurves {
  /** Width, or all axes when `separate` is false. */
  x: LunaCurve
  y: LunaCurve
  z: LunaCurve
  separate: boolean
}

/** Rotation curves in radians per second. */
export interface LunaRotationCurves {
  x: LunaCurve
  y: LunaCurve
  z: LunaCurve
  separate: boolean
}

export interface LunaSheet {
  /** 0 the whole sheet, 1 a single row. */
  animation: 0 | 1
  tilesX: number
  tilesY: number
  frame: LunaCurve
  startFrame: LunaCurve
  cycles: number
  row: number
  randomRow: boolean
}

export interface LunaNoise {
  separate: boolean
  strengthX: LunaCurve
  strengthY: LunaCurve
  strengthZ: LunaCurve
  frequency: number
  damping: boolean
  scrollSpeed: LunaCurve
  positionAmount: LunaCurve
  sizeAmount: LunaCurve
}

export interface LunaRenderer {
  /** Unity's ParticleSystemRenderMode: 0 billboard, 1 stretch, 2 horizontal, 3 vertical, 4 mesh, 5 none. */
  renderMode: number
  /** 0 view, 1 world, 2 local, 3 facing, 4 velocity. */
  alignment: number
  /** 0 none, 1 distance, 2 oldest in front, 3 youngest in front. */
  sortMode: number
  sortingOrder: number
  lengthScale: number
  velocityScale: number
  minParticleSize: number
  maxParticleSize: number
  applyActiveColorSpace: boolean
}

/** What one particle system uses that Luna cannot reproduce. */
export interface LunaUnsupported {
  /** The module as the Unity inspector names it. */
  module: string
  detail: string
}

/** A GameObject of the prefab. A node comes after its parent in `LunaEffectSpec.nodes`. */
export interface LunaNodeSpec {
  name: string
  /** Index of the parent in `nodes`, or -1 for a root. */
  parent: number
  /** Unity space, local to the parent. */
  position: Vec3
  /** Unity quaternion (x, y, z, w). */
  rotation: readonly [number, number, number, number]
  scale: Vec3
  active: boolean
}

/** One particle system. Every module the asset does not use is absent, and the host turns it off. */
export interface LunaEmitterSpec {
  /** Name of the particle system, for reports. */
  name: string
  /** Index into `LunaEffectSpec.nodes` of the GameObject that holds it. */
  node: number
  /** Index into `LunaEffectSpec.materials`. Emitters share a material by sharing an index. */
  material: number
  main: LunaMain
  emission: LunaEmission
  shape?: LunaShape
  velocity?: LunaVelocity
  inheritVelocity?: { mode: number; curve: LunaCurve }
  force?: LunaForce
  limit?: LunaLimit
  colorOverLifetime?: LunaColor
  colorBySpeed?: { color: LunaColor; min: number; max: number }
  sizeOverLifetime?: LunaSizeCurves
  sizeBySpeed?: LunaSizeCurves & { min: number; max: number }
  rotationOverLifetime?: LunaRotationCurves
  rotationBySpeed?: LunaRotationCurves & { min: number; max: number }
  sheet?: LunaSheet
  noise?: LunaNoise
  renderer: LunaRenderer
  /** Things this system uses that Luna does not do. The host reports each as `unsupported`. */
  unsupported: LunaUnsupported[]
}

export interface LunaEffectSpec {
  /** The project draws in linear colour space. The host must have been started for the same space. */
  linear: boolean
  materials: LunaMaterialSpec[]
  nodes: LunaNodeSpec[]
  emitters: LunaEmitterSpec[]
}

/** Where the camera stands and how it sees, in Unity space. */
export interface LunaCamera {
  position: Vec3
  /** Unity quaternion (x, y, z, w). */
  rotation: readonly [number, number, number, number]
  /** Vertical field of view in degrees. */
  fov: number
  near: number
  far: number
}

export type LunaCommand =
  /** `id` names this load: the host echoes it on `loaded`, `unsupported` and the error of the load, so the app can drop answers of a load it has since replaced. */
  | { type: 'load'; id?: number; effect: LunaEffectSpec }
  | { type: 'camera'; camera: LunaCamera }
  /** The colour behind the effect, as drawn on screen (no colour-space conversion). */
  | { type: 'background'; color: Rgba }
  /** How fast the effect plays: 1 is normal. Applies to what plays now and to later shots. */
  | { type: 'speed'; value: number }
  /** Whether the host starts a finished effect over by itself. On by default. The app turns it off and restarts the effect from its own clock, so both players loop together. */
  | { type: 'loop'; value: boolean }
  | { type: 'play' }
  | { type: 'pause' }
  | { type: 'restart' }
  | { type: 'seek'; time: number }
  | { type: 'fire' }
  | { type: 'dispose' }

export interface LunaStats {
  /** Seconds since the effect started. */
  time: number
  /** Live particles over all emitters. */
  particles: number
  /** Draw calls of the last frame. */
  drawCalls: number
}

export type LunaEvent =
  | { type: 'ready'; version: string }
  /** The effect is built and its textures are handed to the engine. `textureMs` is the time that took. */
  | { type: 'loaded'; textureMs: number; id?: number }
  | ({ type: 'time' } & LunaStats)
  | { type: 'error'; message: string; id?: number }
  /** Something the preview could not reproduce, such as a shader that fell back or a module Luna lacks. `emitter` names the particle system. */
  | { type: 'unsupported'; what: string; detail: string; emitter?: string; id?: number }

/** Marks messages of this protocol, so other `postMessage` traffic is ignored. */
export const LUNA_CHANNEL = 'asset-hub-luna'

export interface LunaEnvelope<T> {
  channel: typeof LUNA_CHANNEL
  message: T
}

export function isEnvelope(data: unknown): data is LunaEnvelope<{ type: string }> {
  if (typeof data !== 'object' || data === null) return false
  const d = data as { channel?: unknown; message?: { type?: unknown } }
  return d.channel === LUNA_CHANNEL && typeof d.message?.type === 'string'
}

export const envelope = <T>(message: T): LunaEnvelope<T> => ({ channel: LUNA_CHANNEL, message })
