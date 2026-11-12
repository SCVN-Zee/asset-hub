/**
 * Turns a built effect (the particle systems we parsed, the GameObjects that hold them, their materials) into the
 * serialisable description the Luna host applies (`LunaEffectSpec`). Pure: it reads no files and touches no Luna API.
 *
 * Luna is a size-limited runtime. It has no sub emitters, trails, collision, triggers, lights, custom data or external
 * forces, and it ignores some module settings. Where the asset uses one of those, the system's `unsupported` list names
 * it, and the setting is left at its neutral value so that the preview shows what a Luna build shows.
 */
import type { CurveKey, Gradient, MinMax, MinMaxColor, ParticleEmitter, ParticleNoise, ParticleShape, ParticleSim } from '@shared/types'
import { defaultParticleMaterial } from './materialMap'
import type {
  LunaBurst,
  LunaColor,
  LunaCurve,
  LunaEffectSpec,
  LunaEmission,
  LunaEmitterSpec,
  LunaGradient,
  LunaKey,
  LunaMain,
  LunaMaterialSpec,
  LunaNodeSpec,
  LunaForce,
  LunaLimit,
  LunaNoise,
  LunaRenderer,
  LunaRotationCurves,
  LunaShape,
  LunaSheet,
  LunaSizeCurves,
  LunaUnsupported,
  LunaVelocity
} from './protocol'

/** Shape types Luna implements: sphere, hemisphere, cone, box, cone volume, circle, edge, box shell, box edge, donut, rectangle. */
const LUNA_SHAPES = new Set([0, 2, 4, 5, 8, 10, 12, 15, 16, 17, 18])
/**
 * Older shell shapes are a volume shape with zero thickness. Unity treats them the same, so they map without a loss:
 * sphere shell, hemisphere shell, cone shell, cone volume shell, circle edge.
 */
const SHELL_OF: Readonly<Record<number, number>> = { 1: 0, 3: 2, 7: 4, 9: 8, 11: 10 }
const MESH_SHAPES = new Set([6, 13, 14, 19, 20])

const RENDER_MODES = { billboard: 0, stretched: 1, horizontal: 2, vertical: 3, mesh: 4, none: 5 } as const
const ALIGNMENTS = { view: 0, world: 1, local: 2, facing: 3, velocity: 4 } as const

/** Vertex streams a Luna build provides: position, normal, colour, UV (`ParticleSystemVertexStream` numbers). */
const LUNA_STREAMS = new Set([0, 1, 3, 4])

const constant = (value: number): LunaCurve => ({ mode: 0, scalar: value, min: value })

// ---- curves and gradients ------------------------------------------------------------------------------------------

/** A step tangent arrives as null (JSON has no infinity). */
const tangent = (value: number | null | undefined): number => (typeof value === 'number' && Number.isFinite(value) ? value : Infinity)

const keysOf = (keys: CurveKey[]): LunaKey[] =>
  keys.map((k) => ({ t: k.t, v: k.v[0] ?? 0, inTangent: tangent(k.inS[0]), outTangent: tangent(k.outS[0]) }))

/** A MinMaxCurve. A curve mode without keys is a constant of the scalar. */
export function mapCurve(m: MinMax): LunaCurve {
  const out: LunaCurve = { mode: m.mode, scalar: m.scalar, min: m.min }
  if ((m.mode === 1 || m.mode === 2) && m.curve && m.curve.length > 0) {
    out.curve = keysOf(m.curve)
    if (m.mode === 2 && m.minCurve && m.minCurve.length > 0) out.minCurve = keysOf(m.minCurve)
    else if (m.mode === 2) out.mode = 1
    return out
  }
  return m.mode === 3 ? out : { mode: 0, scalar: m.scalar, min: m.scalar }
}

const mapGradient = (g: Gradient): LunaGradient => ({
  colors: g.colors.map((c) => ({ t: c.t, r: c.r, g: c.g, b: c.b })),
  alphas: g.alphas.map((a) => ({ t: a.t, a: a.a })),
  fixed: g.mode === 1
})

/** A MinMaxGradient. A gradient mode without a gradient is the colour. */
export function mapColor(c: MinMaxColor): LunaColor {
  const out: LunaColor = { mode: c.mode, max: [...c.max], min: [...c.min] }
  if ((c.mode === 1 || c.mode === 3 || c.mode === 4) && c.gradient) {
    out.gradient = mapGradient(c.gradient)
    if (c.mode === 3 && c.minGradient) out.minGradient = mapGradient(c.minGradient)
    else if (c.mode === 3) out.mode = 1
    return out
  }
  return c.mode === 2 ? out : { mode: 0, max: [...c.max], min: [...c.max] }
}

// ---- modules ------------------------------------------------------------------------------------------------------

/** The whole count of a burst: two constants give a range, anything else is the scalar. */
function mapBurst(b: ParticleSim['bursts'][number]): LunaBurst {
  const two = b.count.mode === 3
  const max = Math.max(0, Math.round(b.count.scalar))
  return { time: b.time, min: two ? Math.max(0, Math.round(b.count.min)) : max, max, cycles: Math.max(1, Math.round(b.cycles)), interval: b.interval }
}

function mapMain(sim: ParticleSim): LunaMain {
  const size3 = sim.startSize3D
  const rot3 = sim.startRotation3D
  return {
    duration: sim.duration,
    loop: sim.looping,
    prewarm: sim.prewarm,
    startDelay: mapCurve(sim.startDelay),
    startLifetime: mapCurve(sim.startLifetime),
    startSpeed: mapCurve(sim.startSpeed),
    startSize: size3 ? [mapCurve(sim.startSize), mapCurve(size3[0]), mapCurve(size3[1])] : mapCurve(sim.startSize),
    startRotation: rot3 ? [mapCurve(rot3[0]), mapCurve(rot3[1]), mapCurve(sim.startRotation)] : mapCurve(sim.startRotation),
    startColor: mapColor(sim.startColor),
    gravity: mapCurve(sim.gravity),
    simulationSpace: sim.worldSpace || sim.simulationSpace === 1 ? 1 : 0,
    simulationSpeed: sim.simulationSpeed,
    useUnscaledTime: sim.useUnscaledTime === true,
    scalingMode: sim.scalingMode ?? 1,
    maxParticles: Math.max(1, Math.round(sim.maxParticles)),
    emitterVelocityMode: sim.emitterVelocityMode ?? 0,
    playOnAwake: sim.playOnAwake !== false
  }
}

function mapEmission(sim: ParticleSim): LunaEmission {
  return { rateOverTime: mapCurve(sim.rate), rateOverDistance: sim.rateOverDistance ? mapCurve(sim.rateOverDistance) : constant(0), bursts: sim.bursts.map(mapBurst) }
}

function mapShape(s: ParticleShape): LunaShape {
  const shell = SHELL_OF[s.type]
  const type = shell ?? (LUNA_SHAPES.has(s.type) ? s.type : 0)
  return {
    type,
    angle: s.angle,
    radius: s.radius,
    radiusThickness: shell === undefined ? s.radiusThickness : 0,
    radiusMode: s.radiusMode ?? 0,
    radiusSpread: s.radiusSpread ?? 0,
    length: s.length,
    arc: s.arc,
    arcMode: s.arcMode === 3 ? 0 : (s.arcMode ?? 0),
    arcSpread: s.arcSpread ?? 0,
    arcSpeed: s.arcSpeed ? mapCurve(s.arcSpeed) : constant(1),
    donutRadius: s.donutRadius ?? 0.2,
    position: [...s.position],
    rotation: [...s.rotation],
    scale: [...s.scale],
    randomDirection: s.randomDirection,
    sphericalDirection: s.sphericalDirection ?? 0,
    randomPosition: s.randomPosition ?? 0,
    alignToDirection: s.alignToDirection === true
  }
}

const curves3 = (list: readonly [MinMax, MinMax, MinMax]): [LunaCurve, LunaCurve, LunaCurve] => [mapCurve(list[0]), mapCurve(list[1]), mapCurve(list[2])]
const zero3 = (): [LunaCurve, LunaCurve, LunaCurve] => [constant(0), constant(0), constant(0)]

function mapVelocity(sim: ParticleSim): LunaVelocity | undefined {
  if (!sim.velocity) return undefined
  const [x, y, z] = curves3(sim.velocity)
  const orbit = sim.velocityOrbit
  return {
    x,
    y,
    z,
    world: sim.velocityWorld === true,
    orbital: orbit ? curves3(orbit.orbital) : zero3(),
    orbitalOffset: orbit ? curves3(orbit.offset) : zero3(),
    radial: orbit ? mapCurve(orbit.radial) : constant(0),
    speedModifier: orbit ? mapCurve(orbit.speedModifier) : constant(1)
  }
}

function mapForce(sim: ParticleSim): LunaForce | undefined {
  if (!sim.force) return undefined
  const [x, y, z] = curves3(sim.force)
  return { x, y, z, world: sim.forceWorld === true, randomized: sim.forceRandomized === true }
}

function mapLimit(sim: ParticleSim): LunaLimit | undefined {
  const l = sim.limit
  if (!l) return undefined
  return {
    limit: mapCurve(l.limit),
    dampen: l.dampen,
    drag: l.drag ? mapCurve(l.drag) : constant(0),
    dragBySize: l.dragBySize === true,
    dragByVelocity: l.dragByVelocity === true,
    world: l.world === true
  }
}

/** `main` is the width (or every axis); `rest` are the other two when the module separates the axes. */
function mapAxes(main: MinMax, rest: readonly [MinMax, MinMax] | undefined): LunaSizeCurves {
  const x = mapCurve(main)
  return rest ? { x, y: mapCurve(rest[0]), z: mapCurve(rest[1]), separate: true } : { x, y: x, z: x, separate: false }
}

/** Rotation: `z` is the single curve, `xy` are X and Y when the module separates the axes. */
function mapSpin(z: MinMax, xy: readonly [MinMax, MinMax] | undefined): LunaRotationCurves {
  const cz = mapCurve(z)
  return xy ? { x: mapCurve(xy[0]), y: mapCurve(xy[1]), z: cz, separate: true } : { x: constant(0), y: constant(0), z: cz, separate: false }
}

function mapSheet(sim: ParticleSim): LunaSheet | undefined {
  const s = sim.sheet
  if (!s) return undefined
  return {
    animation: s.singleRow ? 1 : 0,
    tilesX: Math.max(1, Math.round(s.tilesX)),
    tilesY: Math.max(1, Math.round(s.tilesY)),
    frame: mapCurve(s.frame),
    startFrame: mapCurve(s.startFrame),
    cycles: Math.max(1, Math.round(s.cycles)),
    row: Math.max(0, Math.round(s.row)),
    randomRow: s.rowMode === 1
  }
}

function mapNoise(n: ParticleNoise): LunaNoise {
  const x = mapCurve(n.strength)
  const [y, z] = n.separate ? [mapCurve(n.separate[0]), mapCurve(n.separate[1])] : [x, x]
  return {
    separate: n.separate !== undefined,
    strengthX: x,
    strengthY: y,
    strengthZ: z,
    frequency: n.frequency,
    damping: n.damping ?? true,
    // Luna ignores the scroll speed, so it stays at zero (see `unsupportedOf`).
    scrollSpeed: constant(0),
    positionAmount: n.positionAmount ? mapCurve(n.positionAmount) : constant(1),
    sizeAmount: n.sizeAmount ? mapCurve(n.sizeAmount) : constant(0)
  }
}

function mapRenderer(e: ParticleEmitter): LunaRenderer {
  const sim = e.sim
  return {
    // Luna draws mesh particles from a mesh the preview does not load, so they are drawn as billboards.
    renderMode: e.renderMode === 'mesh' ? 0 : RENDER_MODES[e.renderMode],
    alignment: ALIGNMENTS[sim.alignment],
    sortMode: sim.sortMode ?? 0,
    sortingOrder: sim.sortingOrder,
    lengthScale: sim.lengthScale,
    velocityScale: sim.velocityScale,
    minParticleSize: sim.minScreenSize ?? 0,
    maxParticleSize: sim.maxScreenSize,
    applyActiveColorSpace: sim.applyActiveColorSpace !== false
  }
}

const isAnimated = (m: MinMax | undefined): boolean => m !== undefined && (m.mode !== 0 || m.scalar !== 0)

/** What the asset asks of this system that Luna does not do. Each entry stands for one setting Luna leaves out. */
export function unsupportedOf(e: ParticleEmitter): LunaUnsupported[] {
  const sim = e.sim
  const out: LunaUnsupported[] = []
  const add = (module: string, detail: string) => out.push({ module, detail })
  const modules = new Set(e.modules)

  if (modules.has('subEmitters') || (sim.subEmitters?.length ?? 0) > 0) add('Sub Emitters', 'Luna has no sub emitters.')
  if (modules.has('trails') || sim.trail) add('Trails', 'Luna has no trails.')
  if (modules.has('collision')) add('Collision', 'Luna has no particle collision.')
  if (modules.has('lights')) add('Lights', 'Luna has no particle lights.')
  if (modules.has('externalForces')) add('External Forces', 'Luna has no external forces.')
  if (sim.triggers) add('Triggers', 'Luna has no particle triggers.')
  if (sim.custom1 || sim.custom2) add('Custom Data', 'Luna has no custom data.')
  if (sim.customStreams && sim.streams?.some((stream) => !LUNA_STREAMS.has(stream))) add('Renderer', 'Luna has only the position, normal, colour and UV vertex streams.')

  if (sim.simulationSpace === 2) add('Main', 'Custom simulation space is drawn as local space.')
  if (sim.flipRotation) add('Main', 'Flip Rotation is not supported.')

  for (const [i, b] of sim.bursts.entries()) {
    if (b.probability < 1) add('Emission', `Burst ${i} probability ${b.probability} is not supported: the burst always fires.`)
  }

  const shape = sim.shape
  if (shape && (MESH_SHAPES.has(shape.type) || !(LUNA_SHAPES.has(shape.type) || shape.type in SHELL_OF))) {
    add('Shape', `Shape type ${shape.type} is not implemented in Luna: a sphere is used.`)
  }
  if (shape?.arcMode === 3) add('Shape', 'Arc mode Burst Spread is drawn as random.')

  if (sim.limit?.separate) add('Limit Velocity over Lifetime', 'Separate axes are not supported: the combined limit is used.')

  const noise = sim.noise
  if (noise) {
    if ((noise.octaves ?? 1) > 1) add('Noise', 'Octaves are not supported.')
    if (noise.remap) add('Noise', 'Remap is not supported.')
    if (isAnimated(noise.rotationAmount)) add('Noise', 'Rotation amount is not supported.')
    if (isAnimated(noise.scroll) || noise.scrollSpeed !== 0) add('Noise', 'Scroll speed is not supported.')
  }

  const sheet = sim.sheet
  if (sheet) {
    if (sheet.timeMode) add('Texture Sheet Animation', `Time mode ${sheet.timeMode === 1 ? 'speed' : 'FPS'} is not supported: the lifetime is used.`)
    if (sheet.rowMode === 2) add('Texture Sheet Animation', 'Row mode Mesh Index is not supported.')
    if (sheet.flipU || sheet.flipV) add('Texture Sheet Animation', 'Flip U and Flip V are not supported.')
    if (sheet.mode === 1) add('Texture Sheet Animation', 'Sprite mode is drawn as a grid.')
  }

  if (e.renderMode === 'mesh') add('Renderer', 'Mesh particles are drawn as billboards, because the preview does not load the mesh.')
  if ((e.renderMode === 'horizontal' || e.renderMode === 'vertical') && (isAnimated(sim.startRotation) || sim.rotationOverLifetime)) {
    add('Renderer', 'Rotation of horizontal and vertical billboards is not supported.')
  }
  if (sim.alignment === 'velocity' && e.renderMode !== 'stretched') add('Renderer', 'Luna draws nothing for velocity alignment without stretch.')
  if (sim.pivot.some((v) => v !== 0)) add('Renderer', 'The pivot is not supported.')
  if (sim.flip) add('Renderer', 'Flip is not supported.')
  return out
}

function mapEmitter(e: ParticleEmitter, node: number, material: number): LunaEmitterSpec {
  const sim = e.sim
  const spec: LunaEmitterSpec = {
    name: e.name,
    node,
    material,
    main: mapMain(sim),
    emission: mapEmission(sim),
    renderer: mapRenderer(e),
    unsupported: unsupportedOf(e)
  }
  if (sim.shape) spec.shape = mapShape(sim.shape)
  const velocity = mapVelocity(sim)
  if (velocity) spec.velocity = velocity
  if (sim.inheritVelocity) spec.inheritVelocity = { mode: sim.inheritVelocity.mode, curve: mapCurve(sim.inheritVelocity.curve) }
  const force = mapForce(sim)
  if (force) spec.force = force
  const limit = mapLimit(sim)
  if (limit) spec.limit = limit
  if (sim.colorOverLifetime) spec.colorOverLifetime = mapColor(sim.colorOverLifetime)
  if (sim.colorBySpeed) spec.colorBySpeed = { color: mapColor(sim.colorBySpeed.color), min: sim.colorBySpeed.range[0], max: sim.colorBySpeed.range[1] }
  if (sim.sizeOverLifetime) spec.sizeOverLifetime = mapAxes(sim.sizeOverLifetime, sim.sizeOverLifetime3D)
  if (sim.sizeBySpeed) spec.sizeBySpeed = { ...mapAxes(sim.sizeBySpeed.size, sim.sizeBySpeed.separate), min: sim.sizeBySpeed.range[0], max: sim.sizeBySpeed.range[1] }
  if (sim.rotationOverLifetime) spec.rotationOverLifetime = mapSpin(sim.rotationOverLifetime, sim.rotationOverLifetime3D)
  if (sim.rotationBySpeed) spec.rotationBySpeed = { ...mapSpin(sim.rotationBySpeed.z, sim.rotationBySpeed.xy), min: sim.rotationBySpeed.range[0], max: sim.rotationBySpeed.range[1] }
  const sheet = mapSheet(sim)
  if (sheet) spec.sheet = sheet
  if (sim.noise) spec.noise = mapNoise(sim.noise)
  return spec
}

// ---- the effect ---------------------------------------------------------------------------------------------------

export interface BridgeEmitter {
  emitter: ParticleEmitter
  /** Index into the effect's `nodes` of the GameObject that holds the system. */
  node: number
  /** The mapped material. Emitters that hand over the same object share one material. No material draws Unity's Default-Particle. */
  material?: LunaMaterialSpec
}

export interface BridgeInput {
  /** GameObjects in Unity space. A node comes after its parent. */
  nodes: readonly LunaNodeSpec[]
  emitters: readonly BridgeEmitter[]
  /** The project draws in linear colour space. */
  linear: boolean
}

/** Builds the description the host applies. Throws when an emitter or a node points outside the lists it came with. */
export function buildLunaEffect(input: BridgeInput): LunaEffectSpec {
  input.nodes.forEach((n, i) => {
    if (!Number.isInteger(n.parent) || n.parent < -1 || n.parent >= i) throw new Error(`Node ${n.name} must come after its parent (parent ${n.parent}, index ${i})`)
  })
  const materials: LunaMaterialSpec[] = []
  const indexOf = new Map<LunaMaterialSpec, number>()
  const materialIndex = (m: LunaMaterialSpec): number => {
    let i = indexOf.get(m)
    if (i === undefined) {
      i = materials.length
      materials.push(m)
      indexOf.set(m, i)
    }
    return i
  }
  const fallback = defaultParticleMaterial()
  const emitters = input.emitters.map(({ emitter, node, material }) => {
    if (!Number.isInteger(node) || node < 0 || node >= input.nodes.length) throw new Error(`Emitter ${emitter.name} names node ${node}, but the effect has ${input.nodes.length}`)
    return mapEmitter(emitter, node, materialIndex(material ?? fallback))
  })
  return { linear: input.linear, materials, nodes: input.nodes.map((n) => ({ ...n, position: [...n.position], rotation: [...n.rotation], scale: [...n.scale] })), emitters }
}

// ---- hierarchy from three.js ----------------------------------------------------------------------------------------

/** What the bridge reads of a three.js `Object3D`. */
export interface TransformLike {
  name: string
  parent: TransformLike | null
  position: { x: number; y: number; z: number }
  quaternion: { x: number; y: number; z: number; w: number }
  scale: { x: number; y: number; z: number }
  visible: boolean
}

/** Nesting this deep is a loop, not a prefab. */
const MAX_DEPTH = 64

/**
 * The GameObjects between `root` (the group the prefab was built into, which stands for the world) and the given
 * objects, parents first. The prefab build mirrors X to suit three.js, and this undoes it: Luna works in Unity's space.
 */
export function nodesOf(objects: readonly TransformLike[], root: TransformLike): { nodes: LunaNodeSpec[]; indexOf: ReadonlyMap<TransformLike, number> } {
  const nodes: LunaNodeSpec[] = []
  const indexOf = new Map<TransformLike, number>()
  const add = (object: TransformLike, depth: number): number => {
    const known = indexOf.get(object)
    if (known !== undefined) return known
    if (depth > MAX_DEPTH) throw new Error(`The hierarchy of ${object.name} is deeper than ${MAX_DEPTH} levels`)
    const parent = object.parent && object.parent !== root ? add(object.parent, depth + 1) : -1
    const { position: p, quaternion: q, scale: s } = object
    nodes.push({ name: object.name, parent, position: [-p.x, p.y, p.z], rotation: [q.x, -q.y, -q.z, q.w], scale: [s.x, s.y, s.z], active: object.visible })
    indexOf.set(object, nodes.length - 1)
    return nodes.length - 1
  }
  for (const object of objects) if (object !== root) add(object, 0)
  return { nodes, indexOf }
}
