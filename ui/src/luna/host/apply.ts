/**
 * Applies the description of a particle effect (`LunaEffectSpec`) to Luna objects: the node hierarchy, and every
 * module of every ParticleSystem. A new Luna ParticleSystem starts with maxParticles 10, prewarm on and zero size,
 * colour and cone, so each field of each module is set here, and each module the asset leaves out is turned off.
 * Luna internals used here are listed in the header of `boot.ts`.
 */
import type {
  LunaColor,
  LunaCurve,
  LunaEmitterSpec,
  LunaGradient,
  LunaKey,
  LunaNodeSpec,
  LunaRotationCurves,
  LunaSizeCurves,
  Vec3
} from '../protocol'
import type { Dyn } from './boot'

const w = window as unknown as Record<string, Dyn>

/** The modules that a spec can leave out. Each is switched off when the spec has none. */
const OPTIONAL_MODULES = [
  ['shape', 'shape'],
  ['velocity', 'velocityOverLifetime'],
  ['inheritVelocity', 'inheritVelocity'],
  ['force', 'forceOverLifetime'],
  ['limit', 'limitVelocityOverLifetime'],
  ['colorOverLifetime', 'colorOverLifetime'],
  ['colorBySpeed', 'colorBySpeed'],
  ['sizeOverLifetime', 'sizeOverLifetime'],
  ['sizeBySpeed', 'sizeBySpeed'],
  ['rotationOverLifetime', 'rotationOverLifetime'],
  ['rotationBySpeed', 'rotationBySpeed'],
  ['sheet', 'textureSheetAnimation'],
  ['noise', 'noise']
] as const

const vec3 = (v: Vec3): Dyn => new w.pc.Vec3(v[0], v[1], v[2])

/**
 * Makes the GameObjects of the prefab hierarchy. A node comes after its parent, so one pass is enough. Each one is added
 * to `made` as soon as it exists, so a caller that sees a throw can still destroy what was built.
 */
export function createNodes(nodes: readonly LunaNodeSpec[], made: Dyn[] = []): Dyn[] {
  const { UnityEngine, pc } = w
  for (const n of nodes) {
    const go = new UnityEngine.GameObject.$ctor2(n.name)
    made.push(go)
    const t = go.transform
    if (n.parent >= 0) t.parent = made[n.parent].transform
    t.localPosition = vec3(n.position)
    t.localRotation = new pc.Quat(n.rotation[0], n.rotation[1], n.rotation[2], n.rotation[3])
    t.localScale = vec3(n.scale)
  }
  return made
}

/** Turns off the nodes the prefab keeps inactive. Done after the systems exist, so a system on an inactive node is still configured. */
export function activateNodes(nodes: readonly LunaNodeSpec[], made: readonly Dyn[]): void {
  nodes.forEach((n, i) => {
    if (!n.active) made[i].SetActive(false)
  })
}

function animationCurve(keys: readonly LunaKey[]): Dyn {
  // A key is [time, value, inTangent, outTangent, tangentMode]. Mode 0 keeps the tangents as given, and a step is an infinite tangent.
  return new w.pc.AnimationCurve({ keys: keys.map((k) => [k.t, k.v, k.inTangent, k.outTangent, 0]) })
}

/** Writes a MinMaxCurve in place. The module properties return the live curve object, so the fields are set on it. */
export function setCurve(target: Dyn, c: LunaCurve): void {
  target.mode = c.mode
  switch (c.mode) {
    case 0:
      target.constant = c.scalar
      target.constantMax = c.scalar
      target.constantMin = c.scalar
      break
    case 3:
      target.constantMax = c.scalar
      target.constantMin = c.min
      break
    case 1:
      target.curve = animationCurve(c.curve ?? [])
      target.curveMultiplier = c.scalar
      break
    case 2:
      target.curveMax = animationCurve(c.curve ?? [])
      target.curveMin = animationCurve(c.minCurve ?? c.curve ?? [])
      target.curveMultiplier = c.scalar
      break
  }
}

function colorGradient(g: LunaGradient): Dyn {
  // The flat form: five numbers per colour key (time, r, g, b, unused alpha), two per alpha key (time, alpha). Mode 1 is Fixed.
  const colors = g.colors.flatMap((k) => [k.t, k.r, k.g, k.b, 1])
  const alphas = g.alphas.flatMap((k) => [k.t, k.a])
  return new w.pc.ColorGradient(colors, alphas, g.fixed ? 1 : 0)
}

/** Writes a MinMaxGradient in place. */
export function setColor(target: Dyn, c: LunaColor): void {
  target.internalMode = c.mode
  target._colorMax.set(c.max[0], c.max[1], c.max[2], c.max[3])
  target._colorMin.set(c.min[0], c.min[1], c.min[2], c.min[3])
  if (c.gradient) target.gradient = colorGradient(c.gradient)
  if (c.minGradient) target.gradientMin = colorGradient(c.minGradient)
}

function applyMain(ps: Dyn, e: LunaEmitterSpec, loop: boolean): void {
  const m = e.main
  const main = ps.main
  main.duration = m.duration
  main.loop = loop
  main.prewarm = m.prewarm
  setCurve(main.startDelay, m.startDelay)
  setCurve(main.startLifetime, m.startLifetime)
  setCurve(main.startSpeed, m.startSpeed)
  if (Array.isArray(m.startSize)) {
    main.startSize3D = true
    setCurve(main.startSizeX, m.startSize[0])
    setCurve(main.startSizeY, m.startSize[1])
    setCurve(main.startSizeZ, m.startSize[2])
  } else {
    main.startSize3D = false
    setCurve(main.startSize, m.startSize)
  }
  if (Array.isArray(m.startRotation)) {
    main.startRotation3D = true
    setCurve(main.startRotationX, m.startRotation[0])
    setCurve(main.startRotationY, m.startRotation[1])
    setCurve(main.startRotationZ, m.startRotation[2])
  } else {
    main.startRotation3D = false
    setCurve(main.startRotation, m.startRotation)
  }
  setColor(main.startColor, m.startColor)
  setCurve(main.gravityModifier, m.gravity)
  main.simulationSpace = m.simulationSpace
  main.simulationSpeed = m.simulationSpeed
  main.useUnscaledTime = m.useUnscaledTime
  main.scalingMode = m.scalingMode
  main.maxParticles = m.maxParticles
  main.emitterVelocityMode = m.emitterVelocityMode
  main.playOnAwake = m.playOnAwake
}

function applyEmission(ps: Dyn, e: LunaEmitterSpec): void {
  const emission = ps.emission
  emission.enabled = true
  setCurve(emission.rateOverTime, e.emission.rateOverTime)
  setCurve(emission.rateOverDistance, e.emission.rateOverDistance)
  const bursts = e.emission.bursts.map((b) => {
    const burst = new w.pc.ParticleSystemBurst()
    burst.time = b.time
    burst.minCount = b.min
    burst.maxCount = b.max
    burst.cycleCount = b.cycles
    burst.repeatInterval = b.interval
    setCurve(burst.count, b.min === b.max ? { mode: 0, scalar: b.max, min: b.max } : { mode: 3, scalar: b.max, min: b.min })
    return burst
  })
  emission.setBursts(bursts)
}

function applyShape(ps: Dyn, s: NonNullable<LunaEmitterSpec['shape']>): void {
  const shape = ps.shape
  shape.enabled = true
  // The setter builds the shape implementation, and it resets the fields, so it comes first.
  shape.shapeType = s.type
  shape.angle = s.angle
  shape.radius = s.radius
  shape.radiusThickness = s.radiusThickness
  shape.radiusMode = s.radiusMode
  shape.radiusSpread = s.radiusSpread
  setCurve(shape.radiusSpeed, { mode: 0, scalar: 1, min: 1 })
  shape.length = s.length
  shape.arc = s.arc
  shape.arcMode = s.arcMode
  shape.arcSpread = s.arcSpread
  setCurve(shape.arcSpeed, s.arcSpeed)
  shape.donutRadius = s.donutRadius
  shape.position = vec3(s.position)
  shape.rotation = vec3(s.rotation)
  shape.scale = vec3(s.scale)
  shape.randomDirectionAmount = s.randomDirection
  shape.sphericalDirectionAmount = s.sphericalDirection
  shape.randomPositionAmount = s.randomPosition
  shape.alignToDirection = s.alignToDirection
}

function applySize(module: Dyn, s: LunaSizeCurves): void {
  module.separateAxes = s.separate
  if (s.separate) {
    setCurve(module.x, s.x)
    setCurve(module.y, s.y)
    setCurve(module.z, s.z)
  } else {
    setCurve(module.size, s.x)
  }
}

function applyRotation(module: Dyn, r: LunaRotationCurves): void {
  module.separateAxes = r.separate
  setCurve(module.x, r.x)
  setCurve(module.y, r.y)
  setCurve(module.z, r.z)
}

const range = (min: number, max: number): Dyn => new w.pc.Vec2(min, max)

function applyModules(ps: Dyn, e: LunaEmitterSpec): void {
  if (e.shape) applyShape(ps, e.shape)
  if (e.velocity) {
    const v = e.velocity
    const m = ps.velocityOverLifetime
    setCurve(m.x, v.x)
    setCurve(m.y, v.y)
    setCurve(m.z, v.z)
    m.space = v.world ? 1 : 0
    setCurve(m.orbitalX, v.orbital[0])
    setCurve(m.orbitalY, v.orbital[1])
    setCurve(m.orbitalZ, v.orbital[2])
    setCurve(m.orbitalOffsetX, v.orbitalOffset[0])
    setCurve(m.orbitalOffsetY, v.orbitalOffset[1])
    setCurve(m.orbitalOffsetZ, v.orbitalOffset[2])
    setCurve(m.radial, v.radial)
    setCurve(m.speedModifier, v.speedModifier)
  }
  if (e.inheritVelocity) {
    const m = ps.inheritVelocity
    m.mode = e.inheritVelocity.mode
    setCurve(m.curve, e.inheritVelocity.curve)
  }
  if (e.force) {
    const m = ps.forceOverLifetime
    setCurve(m.x, e.force.x)
    setCurve(m.y, e.force.y)
    setCurve(m.z, e.force.z)
    m.space = e.force.world ? 1 : 0
    m.randomized = e.force.randomized
  }
  if (e.limit) {
    const l = e.limit
    const m = ps.limitVelocityOverLifetime
    m.separateAxes = false
    setCurve(m.limit, l.limit)
    m.dampen = l.dampen
    setCurve(m.drag, l.drag)
    m.multiplyDragByParticleSize = l.dragBySize
    m.multiplyDragByParticleVelocity = l.dragByVelocity
    m.space = l.world ? 1 : 0
  }
  if (e.colorOverLifetime) setColor(ps.colorOverLifetime.color, e.colorOverLifetime)
  if (e.colorBySpeed) {
    const m = ps.colorBySpeed
    setColor(m.color, e.colorBySpeed.color)
    m.range = range(e.colorBySpeed.min, e.colorBySpeed.max)
  }
  if (e.sizeOverLifetime) applySize(ps.sizeOverLifetime, e.sizeOverLifetime)
  if (e.sizeBySpeed) {
    applySize(ps.sizeBySpeed, e.sizeBySpeed)
    ps.sizeBySpeed.range = range(e.sizeBySpeed.min, e.sizeBySpeed.max)
  }
  if (e.rotationOverLifetime) applyRotation(ps.rotationOverLifetime, e.rotationOverLifetime)
  if (e.rotationBySpeed) {
    applyRotation(ps.rotationBySpeed, e.rotationBySpeed)
    ps.rotationBySpeed.range = range(e.rotationBySpeed.min, e.rotationBySpeed.max)
  }
  if (e.sheet) {
    const s = e.sheet
    const m = ps.textureSheetAnimation
    m.mode = 0
    m.animation = s.animation
    m.numTilesX = s.tilesX
    m.numTilesY = s.tilesY
    setCurve(m.frameOverTime, s.frame)
    setCurve(m.startFrame, s.startFrame)
    m.cycleCount = s.cycles
    m.rowIndex = s.row
    m.useRandomRow = s.randomRow
    m.flipU = 0
    m.flipV = 0
  }
  if (e.noise) {
    const n = e.noise
    const m = ps.noise
    m.separateAxes = n.separate
    if (n.separate) {
      setCurve(m.strengthX, n.strengthX)
      setCurve(m.strengthY, n.strengthY)
      setCurve(m.strengthZ, n.strengthZ)
    } else {
      setCurve(m.strength, n.strengthX)
    }
    m.frequency = n.frequency
    m.damping = n.damping
    setCurve(m.scrollSpeed, n.scrollSpeed)
    setCurve(m.positionAmount, n.positionAmount)
    setCurve(m.rotationAmount, { mode: 0, scalar: 0, min: 0 })
    setCurve(m.sizeAmount, n.sizeAmount)
    m.octaveCount = 1
    m.octaveMultiplier = 0.5
    m.octaveScale = 2
    m.remapEnabled = false
  }
  for (const [key, module] of OPTIONAL_MODULES) ps[module].enabled = e[key] !== undefined
}

function applyRenderer(renderer: Dyn, e: LunaEmitterSpec, material: Dyn): void {
  const { UnityEngine, pc } = w
  const r = e.renderer
  renderer.sharedMaterial = material
  // The renderer gets its mesh instance only when the mode changes after the material is set.
  renderer.renderMode = UnityEngine.ParticleSystemRenderMode.Mesh
  renderer.renderMode = UnityEngine.ParticleSystemRenderMode.Billboard
  // Mode 5 draws nothing. The renderer is turned off instead of relying on the enum value.
  renderer.enabled = r.renderMode !== 5
  renderer.renderMode = r.renderMode === 5 ? 0 : r.renderMode
  renderer.alignment = r.alignment
  renderer.sortMode = r.sortMode
  renderer.sortingOrder = r.sortingOrder
  renderer.lengthScale = r.lengthScale
  renderer.velocityScale = r.velocityScale
  renderer.cameraVelocityScale = 0
  renderer.normalDirection = 1
  renderer.sortingFudge = 0
  renderer.minParticleSize = r.minParticleSize
  renderer.maxParticleSize = r.maxParticleSize
  renderer.pivot = new pc.Vec3(0, 0, 0)
  renderer.applyActiveColorSpace = r.applyActiveColorSpace
}

/**
 * Adds a ParticleSystem to a node and applies the emitter. The system is left stopped, so the caller decides when it
 * plays. `loop` overrides the asset's loop flag, which a one-off shot turns off.
 */
export function createSystem(node: Dyn, e: LunaEmitterSpec, material: Dyn, loop: boolean): Dyn {
  const ps = node.AddComponent(w.UnityEngine.ParticleSystem)
  // A new system plays at once. Stop it and drop what it made, so Play On Awake decides.
  ps.Stop$2(false, w.UnityEngine.ParticleSystemStopBehavior.StopEmittingAndClear)
  applyRenderer(node.GetComponent(w.UnityEngine.ParticleSystemRenderer), e, material)
  // Main first: the particle cap and the clock fields must be right before the other modules read them.
  applyMain(ps, e, loop)
  applyEmission(ps, e)
  applyModules(ps, e)
  return ps
}
