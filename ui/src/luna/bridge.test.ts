import { describe, expect, it } from 'vitest'
import type { MinMax, MinMaxColor, ParticleEmitter, ParticleSim } from '@shared/types'
import { buildLunaEffect, mapColor, mapCurve, nodesOf, unsupportedOf, type BridgeEmitter, type TransformLike } from './bridge'
import { defaultParticleMaterial, mapMaterial } from './materialMap'
import type { LunaNodeSpec } from './protocol'

const k = (n: number): MinMax => ({ mode: 0, scalar: n, min: n })
const WHITE: MinMaxColor = { mode: 0, max: [1, 1, 1, 1], min: [1, 1, 1, 1] }

function sim(parts: Partial<ParticleSim> = {}): ParticleSim {
  return {
    duration: 5,
    looping: true,
    prewarm: false,
    worldSpace: false,
    simulationSpeed: 1,
    startDelay: k(0),
    startLifetime: k(2),
    startSpeed: k(3),
    startSize: k(1),
    startRotation: k(0),
    startColor: WHITE,
    gravity: k(0),
    maxParticles: 1000,
    rate: k(10),
    bursts: [],
    lengthScale: 2,
    velocityScale: 0,
    pivot: [0, 0, 0],
    maxScreenSize: 0.5,
    sortingOrder: 0,
    sortingFudge: 0,
    alignment: 'view',
    ...parts
  }
}

function emitter(parts: Partial<ParticleEmitter> = {}, simParts: Partial<ParticleSim> = {}): ParticleEmitter {
  return {
    id: '1',
    name: 'Fx',
    duration: 5,
    looping: true,
    prewarm: false,
    lifetime: 2,
    speed: 3,
    size: 1,
    maxParticles: 1000,
    rate: 10,
    bursts: 0,
    peak: 20,
    renderMode: 'billboard',
    modules: [],
    subEmitters: 0,
    sim: sim(simParts),
    ...parts
  }
}

const ROOT: LunaNodeSpec = { name: 'Root', parent: -1, position: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], active: true }
const bridge = (e: ParticleEmitter, material?: BridgeEmitter['material']) => buildLunaEffect({ nodes: [ROOT], emitters: [{ emitter: e, node: 0, material }], linear: false }).emitters[0]

describe('curves', () => {
  it('keeps a constant', () => {
    expect(mapCurve(k(4))).toEqual({ mode: 0, scalar: 4, min: 4 })
  })

  it('keeps two constants as a range', () => {
    expect(mapCurve({ mode: 3, scalar: 5, min: 2 })).toEqual({ mode: 3, scalar: 5, min: 2 })
  })

  it('maps a curve with its multiplier and turns a null tangent into a step', () => {
    const m: MinMax = {
      mode: 1,
      scalar: 2,
      min: 0,
      curve: [
        { t: 0, v: [0], inS: [1], outS: [Infinity] },
        { t: 1, v: [1], inS: [null as unknown as number], outS: [0] }
      ]
    }
    expect(mapCurve(m)).toEqual({
      mode: 1,
      scalar: 2,
      min: 0,
      curve: [
        { t: 0, v: 0, inTangent: 1, outTangent: Infinity },
        { t: 1, v: 1, inTangent: Infinity, outTangent: 0 }
      ]
    })
  })

  it('maps two curves, and falls back to one curve when the lower one is missing', () => {
    const key = (v: number) => [{ t: 0, v: [v], inS: [0], outS: [0] }]
    expect(mapCurve({ mode: 2, scalar: 1, min: 0, curve: key(1), minCurve: key(0.5) }).minCurve).toEqual([{ t: 0, v: 0.5, inTangent: 0, outTangent: 0 }])
    expect(mapCurve({ mode: 2, scalar: 1, min: 0, curve: key(1) }).mode).toBe(1)
  })

  it('draws a curve mode without keys as its constant', () => {
    expect(mapCurve({ mode: 1, scalar: 3, min: 0 })).toEqual({ mode: 0, scalar: 3, min: 3 })
  })
})

describe('gradients', () => {
  const gradient = { colors: [{ t: 0, r: 1, g: 0, b: 0 }, { t: 1, r: 0, g: 0, b: 1 }], alphas: [{ t: 0, a: 1 }, { t: 1, a: 0 }] }

  it('keeps a plain colour', () => {
    expect(mapColor(WHITE)).toEqual({ mode: 0, max: [1, 1, 1, 1], min: [1, 1, 1, 1] })
  })

  it('maps a gradient with its keys and the blend mode', () => {
    const out = mapColor({ ...WHITE, mode: 1, gradient })
    expect(out.gradient).toEqual({ ...gradient, fixed: false })
    expect(mapColor({ ...WHITE, mode: 1, gradient: { ...gradient, mode: 1 } }).gradient?.fixed).toBe(true)
  })

  it('maps two gradients and the random colour of a gradient', () => {
    expect(mapColor({ ...WHITE, mode: 3, gradient, minGradient: gradient }).minGradient).toBeDefined()
    expect(mapColor({ ...WHITE, mode: 4, gradient }).mode).toBe(4)
  })

  it('falls back to a lone gradient or to the colour when a gradient is missing', () => {
    expect(mapColor({ ...WHITE, mode: 3, gradient }).mode).toBe(1)
    expect(mapColor({ ...WHITE, mode: 1, max: [0.5, 0.5, 0.5, 1] })).toMatchObject({ mode: 0, max: [0.5, 0.5, 0.5, 1] })
  })

  it('keeps two colours', () => {
    expect(mapColor({ mode: 2, max: [1, 0, 0, 1], min: [0, 0, 1, 1] })).toEqual({ mode: 2, max: [1, 0, 0, 1], min: [0, 0, 1, 1] })
  })
})

describe('main module', () => {
  it('copies every start value', () => {
    const { main } = bridge(emitter({}, { duration: 3, looping: false, prewarm: true, simulationSpeed: 2, maxParticles: 77, gravity: k(0.5), startDelay: k(1) }))
    expect(main).toMatchObject({ duration: 3, loop: false, prewarm: true, simulationSpeed: 2, maxParticles: 77, playOnAwake: true, scalingMode: 1, emitterVelocityMode: 0, simulationSpace: 0, useUnscaledTime: false })
    expect(main.gravity.scalar).toBe(0.5)
    expect(main.startDelay.scalar).toBe(1)
    expect(main.startLifetime.scalar).toBe(2)
    expect(main.startSpeed.scalar).toBe(3)
  })

  it('uses three size curves when the size is 3D, and one otherwise', () => {
    expect(Array.isArray(bridge(emitter()).main.startSize)).toBe(false)
    const size = bridge(emitter({}, { startSize: k(1), startSize3D: [k(2), k(3)] })).main.startSize
    expect(Array.isArray(size) && size.map((c) => c.scalar)).toEqual([1, 2, 3])
  })

  it('uses three rotation curves when the rotation is 3D, with Z last', () => {
    const rotation = bridge(emitter({}, { startRotation: k(0.3), startRotation3D: [k(0.1), k(0.2)] })).main.startRotation
    expect(Array.isArray(rotation) && rotation.map((c) => c.scalar)).toEqual([0.1, 0.2, 0.3])
  })

  it('reads the extra main fields', () => {
    const { main } = bridge(emitter({}, { simulationSpace: 1, scalingMode: 2, emitterVelocityMode: 1, playOnAwake: false, useUnscaledTime: true }))
    expect(main).toMatchObject({ simulationSpace: 1, scalingMode: 2, emitterVelocityMode: 1, playOnAwake: false, useUnscaledTime: true })
  })

  it('puts a world space system in world space', () => {
    expect(bridge(emitter({}, { worldSpace: true })).main.simulationSpace).toBe(1)
  })

  it('keeps at least one particle', () => {
    expect(bridge(emitter({}, { maxParticles: 0 })).main.maxParticles).toBe(1)
  })
})

describe('emission', () => {
  it('maps the rates and rounds burst counts', () => {
    const { emission } = bridge(
      emitter({}, {
        rate: k(12),
        rateOverDistance: k(4),
        bursts: [
          { time: 0.5, count: { mode: 3, scalar: 9.4, min: 3.2 }, cycles: 3, interval: 0.25, probability: 1 },
          { time: 1, count: k(7), cycles: 1, interval: 0.01, probability: 1 }
        ]
      })
    )
    expect(emission.rateOverTime.scalar).toBe(12)
    expect(emission.rateOverDistance.scalar).toBe(4)
    expect(emission.bursts).toEqual([
      { time: 0.5, min: 3, max: 9, cycles: 3, interval: 0.25 },
      { time: 1, min: 7, max: 7, cycles: 1, interval: 0.01 }
    ])
  })

  it('has no distance rate unless the asset sets one', () => {
    expect(bridge(emitter()).emission.rateOverDistance).toEqual({ mode: 0, scalar: 0, min: 0 })
  })
})

describe('shape', () => {
  const shape = (parts: Partial<NonNullable<ParticleSim['shape']>> = {}): NonNullable<ParticleSim['shape']> => ({
    type: 4,
    radius: 2,
    radiusThickness: 1,
    angle: 30,
    arc: 270,
    length: 5,
    position: [1, 2, 3],
    rotation: [0, 90, 0],
    scale: [1, 2, 1],
    randomDirection: 0.1,
    ...parts
  })

  it('has no shape when the module is off', () => {
    expect(bridge(emitter()).shape).toBeUndefined()
  })

  it('copies the cone settings and the shape transform', () => {
    expect(bridge(emitter({}, { shape: shape() })).shape).toMatchObject({
      type: 4,
      radius: 2,
      angle: 30,
      arc: 270,
      length: 5,
      radiusThickness: 1,
      position: [1, 2, 3],
      rotation: [0, 90, 0],
      scale: [1, 2, 1],
      randomDirection: 0.1,
      donutRadius: 0.2,
      alignToDirection: false
    })
  })

  it.each([0, 2, 4, 5, 8, 10, 12, 15, 16, 17, 18])('keeps shape type %i, which Luna implements', (type) => {
    expect(bridge(emitter({}, { shape: shape({ type }) })).shape?.type).toBe(type)
    expect(unsupportedOf(emitter({}, { shape: shape({ type }) }))).toEqual([])
  })

  it.each([
    [1, 0],
    [3, 2],
    [7, 4],
    [9, 8],
    [11, 10]
  ])('draws the shell shape %i as shape %i with a zero thickness, and reports nothing', (type, to) => {
    const e = emitter({}, { shape: shape({ type, radiusThickness: 1 }) })
    expect(bridge(e).shape).toMatchObject({ type: to, radiusThickness: 0 })
    expect(unsupportedOf(e)).toEqual([])
  })

  it.each([6, 13, 14, 19, 20, 99])('reports shape type %i, which Luna lacks, and uses a sphere', (type) => {
    const e = emitter({}, { shape: shape({ type }) })
    expect(bridge(e).shape?.type).toBe(0)
    expect(unsupportedOf(e).map((u) => u.module)).toEqual(['Shape'])
  })

  it('reads the walk modes, the donut radius and the direction settings', () => {
    const s = bridge(
      emitter({}, {
        shape: shape({ type: 17, radiusMode: 2, radiusSpread: 0.5, arcMode: 1, arcSpread: 0.25, arcSpeed: k(3), donutRadius: 0.4, sphericalDirection: 0.3, randomPosition: 0.2, alignToDirection: true })
      })
    ).shape
    expect(s).toMatchObject({ radiusMode: 2, radiusSpread: 0.5, arcMode: 1, arcSpread: 0.25, donutRadius: 0.4, sphericalDirection: 0.3, randomPosition: 0.2, alignToDirection: true })
    expect(s?.arcSpeed.scalar).toBe(3)
  })

  it('draws arc mode burst spread as random and says so', () => {
    const e = emitter({}, { shape: shape({ arcMode: 3 }) })
    expect(bridge(e).shape?.arcMode).toBe(0)
    expect(unsupportedOf(e).map((u) => u.detail)).toEqual(['Arc mode Burst Spread is drawn as random.'])
  })
})

describe('velocity, force and limit', () => {
  it('maps velocity over lifetime with its orbital part', () => {
    const v = bridge(
      emitter({}, {
        velocity: [k(1), k(2), k(3)],
        velocityWorld: true,
        velocityOrbit: { orbital: [k(0), k(4), k(0)], offset: [k(0), k(0), k(5)], radial: k(6), speedModifier: k(0.5) }
      })
    ).velocity
    expect([v?.x.scalar, v?.y.scalar, v?.z.scalar]).toEqual([1, 2, 3])
    expect(v?.world).toBe(true)
    expect(v?.orbital[1].scalar).toBe(4)
    expect(v?.orbitalOffset[2].scalar).toBe(5)
    expect(v?.radial.scalar).toBe(6)
    expect(v?.speedModifier.scalar).toBe(0.5)
  })

  it('gives plain velocity a neutral orbit and speed modifier', () => {
    const v = bridge(emitter({}, { velocity: [k(1), k(0), k(0)] })).velocity
    expect(v?.world).toBe(false)
    expect(v?.orbital.map((c) => c.scalar)).toEqual([0, 0, 0])
    expect(v?.speedModifier.scalar).toBe(1)
  })

  it('maps force with its space and random flag', () => {
    expect(bridge(emitter({}, { force: [k(0), k(-9), k(0)], forceWorld: true, forceRandomized: true })).force).toMatchObject({ world: true, randomized: true })
    expect(bridge(emitter({}, { force: [k(0), k(-9), k(0)] })).force).toMatchObject({ world: false, randomized: false })
  })

  it('maps the limit with drag and space', () => {
    const l = bridge(emitter({}, { limit: { limit: k(2), dampen: 0.4, drag: k(0.7), dragBySize: true, dragByVelocity: true, world: true } })).limit
    expect(l).toMatchObject({ dampen: 0.4, dragBySize: true, dragByVelocity: true, world: true })
    expect(l?.limit.scalar).toBe(2)
    expect(l?.drag.scalar).toBe(0.7)
  })

  it('maps inherit velocity', () => {
    expect(bridge(emitter({}, { inheritVelocity: { mode: 1, curve: k(0.5) } })).inheritVelocity).toMatchObject({ mode: 1, curve: { scalar: 0.5 } })
  })

  it('reports separate limit axes and keeps the combined limit', () => {
    const e = emitter({}, { limit: { limit: k(2), dampen: 1, separate: [k(1), k(2), k(3)] } })
    expect(bridge(e).limit?.limit.scalar).toBe(2)
    expect(unsupportedOf(e).map((u) => u.module)).toEqual(['Limit Velocity over Lifetime'])
  })
})

describe('over lifetime and by speed', () => {
  it('maps colour over lifetime and colour by speed', () => {
    const e = bridge(emitter({}, { colorOverLifetime: WHITE, colorBySpeed: { color: WHITE, range: [1, 5] } }))
    expect(e.colorOverLifetime?.mode).toBe(0)
    expect(e.colorBySpeed).toMatchObject({ min: 1, max: 5 })
  })

  it('maps one size curve to every axis, and separate axes apart', () => {
    const one = bridge(emitter({}, { sizeOverLifetime: k(2) })).sizeOverLifetime
    expect(one).toMatchObject({ separate: false })
    expect([one?.x.scalar, one?.y.scalar, one?.z.scalar]).toEqual([2, 2, 2])
    const apart = bridge(emitter({}, { sizeOverLifetime: k(1), sizeOverLifetime3D: [k(2), k(3)] })).sizeOverLifetime
    expect(apart).toMatchObject({ separate: true })
    expect([apart?.x.scalar, apart?.y.scalar, apart?.z.scalar]).toEqual([1, 2, 3])
  })

  it('maps rotation over lifetime with Z as the single curve', () => {
    const one = bridge(emitter({}, { rotationOverLifetime: k(1.5) })).rotationOverLifetime
    expect(one).toMatchObject({ separate: false })
    expect(one?.z.scalar).toBe(1.5)
    const apart = bridge(emitter({}, { rotationOverLifetime: k(3), rotationOverLifetime3D: [k(1), k(2)] })).rotationOverLifetime
    expect([apart?.x.scalar, apart?.y.scalar, apart?.z.scalar]).toEqual([1, 2, 3])
  })

  it('maps size and rotation by speed with their ranges', () => {
    const e = bridge(emitter({}, { sizeBySpeed: { size: k(1), separate: [k(2), k(3)], range: [0, 4] }, rotationBySpeed: { z: k(5), xy: [k(6), k(7)], range: [1, 2] } }))
    expect(e.sizeBySpeed).toMatchObject({ separate: true, min: 0, max: 4 })
    expect(e.rotationBySpeed).toMatchObject({ separate: true, min: 1, max: 2 })
    expect([e.rotationBySpeed?.x.scalar, e.rotationBySpeed?.y.scalar, e.rotationBySpeed?.z.scalar]).toEqual([6, 7, 5])
  })

  it('leaves out the modules the asset does not use', () => {
    const e = bridge(emitter())
    for (const key of ['shape', 'velocity', 'force', 'limit', 'colorOverLifetime', 'colorBySpeed', 'sizeOverLifetime', 'sizeBySpeed', 'rotationOverLifetime', 'rotationBySpeed', 'sheet', 'noise', 'inheritVelocity']) {
      expect(e, key).not.toHaveProperty(key)
    }
  })
})

describe('texture sheet and noise', () => {
  it('maps the sheet grid, frame curves and random row', () => {
    const sheet = bridge(emitter({}, { sheet: { tilesX: 4, tilesY: 2, singleRow: true, row: 1, frame: k(1), startFrame: k(0.25), cycles: 2, rowMode: 1 } })).sheet
    expect(sheet).toMatchObject({ animation: 1, tilesX: 4, tilesY: 2, row: 1, cycles: 2, randomRow: true })
    expect(sheet?.startFrame.scalar).toBe(0.25)
  })

  it('maps a whole-sheet animation', () => {
    expect(bridge(emitter({}, { sheet: { tilesX: 4, tilesY: 4, singleRow: false, row: 0, frame: k(1), startFrame: k(0), cycles: 1 } })).sheet).toMatchObject({ animation: 0, randomRow: false })
  })

  it('maps noise with separate axes, damping and amounts', () => {
    const n = bridge(emitter({}, { noise: { strength: k(1), frequency: 0.7, scrollSpeed: 0, separate: [k(2), k(3)], damping: false, positionAmount: k(0.5), sizeAmount: k(0.2) } })).noise
    expect(n).toMatchObject({ separate: true, frequency: 0.7, damping: false })
    expect([n?.strengthX.scalar, n?.strengthY.scalar, n?.strengthZ.scalar]).toEqual([1, 2, 3])
    expect([n?.positionAmount.scalar, n?.sizeAmount.scalar]).toEqual([0.5, 0.2])
  })

  it('gives plain noise unit strength on every axis, damping on and a full position amount', () => {
    const n = bridge(emitter({}, { noise: { strength: k(2), frequency: 1, scrollSpeed: 0 } })).noise
    expect([n?.strengthX.scalar, n?.strengthY.scalar, n?.strengthZ.scalar]).toEqual([2, 2, 2])
    expect(n).toMatchObject({ separate: false, damping: true })
    expect([n?.positionAmount.scalar, n?.sizeAmount.scalar]).toEqual([1, 0])
  })

  it('keeps the scroll speed at zero, because Luna ignores it', () => {
    expect(bridge(emitter({}, { noise: { strength: k(1), frequency: 1, scrollSpeed: 0, scroll: k(2) } })).noise?.scrollSpeed.scalar).toBe(0)
  })
})

describe('renderer', () => {
  it.each([
    ['billboard', 0],
    ['stretched', 1],
    ['horizontal', 2],
    ['vertical', 3],
    ['none', 5]
  ] as const)('maps render mode %s to %i', (renderMode, number) => {
    expect(bridge(emitter({ renderMode })).renderer.renderMode).toBe(number)
  })

  it.each([
    ['view', 0],
    ['world', 1],
    ['local', 2],
    ['facing', 3],
    ['velocity', 4]
  ] as const)('maps alignment %s to %i', (alignment, number) => {
    expect(bridge(emitter({}, { alignment })).renderer.alignment).toBe(number)
  })

  it('copies sorting, stretch and size limits', () => {
    expect(bridge(emitter({}, { sortMode: 2, sortingOrder: 5, lengthScale: 3, velocityScale: 0.2, minScreenSize: 0.1, maxScreenSize: 0.8, applyActiveColorSpace: false })).renderer).toEqual({
      renderMode: 0,
      alignment: 0,
      sortMode: 2,
      sortingOrder: 5,
      lengthScale: 3,
      velocityScale: 0.2,
      minParticleSize: 0.1,
      maxParticleSize: 0.8,
      applyActiveColorSpace: false
    })
  })

  it('draws mesh particles as billboards and says why', () => {
    const e = emitter({ renderMode: 'mesh' })
    expect(bridge(e).renderer.renderMode).toBe(0)
    expect(unsupportedOf(e).map((u) => u.module)).toEqual(['Renderer'])
  })
})

describe('unsupported modules', () => {
  const modules = (e: ParticleEmitter) => unsupportedOf(e).map((u) => u.module)

  it('reports nothing for a plain system', () => {
    expect(unsupportedOf(emitter())).toEqual([])
  })

  it.each([
    ['subEmitters', 'Sub Emitters'],
    ['trails', 'Trails'],
    ['collision', 'Collision'],
    ['lights', 'Lights'],
    ['externalForces', 'External Forces']
  ] as const)('reports the %s module', (module, name) => {
    expect(modules(emitter({ modules: [module] }))).toEqual([name])
  })

  it('reports a sub emitter list and a parsed trail even when the module list lacks them', () => {
    expect(modules(emitter({}, { subEmitters: [{ id: '2', type: 'birth', inherit: 0, probability: 1 }] }))).toEqual(['Sub Emitters'])
    expect(modules(emitter({}, { trail: {} as NonNullable<ParticleSim['trail']> }))).toEqual(['Trails'])
  })

  it('does not report the modules Luna has', () => {
    expect(modules(emitter({ modules: ['noise', 'textureSheet', 'velocity', 'force'] }))).toEqual([])
  })

  it('reports triggers, custom data and custom vertex streams', () => {
    expect(modules(emitter({}, { triggers: true }))).toEqual(['Triggers'])
    expect(modules(emitter({}, { custom1: [k(1)] }))).toEqual(['Custom Data'])
    expect(modules(emitter({}, { custom2: [k(1)] }))).toEqual(['Custom Data'])
    expect(modules(emitter({}, { streams: [0, 1, 3, 4, 11], customStreams: true }))).toEqual(['Renderer'])
    expect(modules(emitter({}, { streams: [0, 1, 3, 4], customStreams: true }))).toEqual([])
    // A stream list that the renderer does not use counts for nothing.
    expect(modules(emitter({}, { streams: [0, 1, 3, 4, 5] }))).toEqual([])
  })

  it('reports custom simulation space, flip rotation and burst probability', () => {
    const e = emitter({}, { simulationSpace: 2, flipRotation: 0.5, bursts: [{ time: 0, count: k(5), cycles: 1, interval: 0.01, probability: 0.5 }] })
    expect(modules(e)).toEqual(['Main', 'Main', 'Emission'])
    expect(bridge(e).main.simulationSpace).toBe(0)
  })

  it('reports the noise settings Luna ignores, and only those', () => {
    const noise = { strength: k(1), frequency: 1, scrollSpeed: 0 }
    expect(modules(emitter({}, { noise }))).toEqual([])
    const e = emitter({}, { noise: { ...noise, octaves: 3, remap: [k(1), k(1), k(1)], rotationAmount: k(1), scroll: k(2) } })
    expect(unsupportedOf(e).map((u) => u.detail)).toEqual(['Octaves are not supported.', 'Remap is not supported.', 'Rotation amount is not supported.', 'Scroll speed is not supported.'])
  })

  it('reports the sheet settings Luna ignores', () => {
    const sheet = { tilesX: 2, tilesY: 2, singleRow: false, row: 0, frame: k(1), startFrame: k(0), cycles: 1 }
    expect(modules(emitter({}, { sheet }))).toEqual([])
    expect(unsupportedOf(emitter({}, { sheet: { ...sheet, timeMode: 2, rowMode: 2, flipU: 1, mode: 1 } })).map((u) => u.module)).toEqual(Array(4).fill('Texture Sheet Animation'))
  })

  it('reports renderer settings Luna ignores', () => {
    expect(modules(emitter({}, { pivot: [0, 0.5, 0] }))).toEqual(['Renderer'])
    expect(modules(emitter({}, { flip: [1, 0, 0] }))).toEqual(['Renderer'])
    expect(modules(emitter({}, { alignment: 'velocity' }))).toEqual(['Renderer'])
    expect(modules(emitter({ renderMode: 'stretched' }, { alignment: 'velocity' }))).toEqual([])
    expect(modules(emitter({ renderMode: 'horizontal' }, { startRotation: k(1) }))).toEqual(['Renderer'])
  })

  it('lists each system apart in the effect, under its own name', () => {
    const plain = emitter({ name: 'Plain' })
    const ribbon = emitter({ name: 'Ribbon', modules: ['trails', 'collision'] })
    const out = buildLunaEffect({ nodes: [ROOT], emitters: [{ emitter: plain, node: 0 }, { emitter: ribbon, node: 0 }], linear: false })
    expect(out.emitters.map((e) => [e.name, e.unsupported.map((u) => u.module)])).toEqual([
      ['Plain', []],
      ['Ribbon', ['Trails', 'Collision']]
    ])
  })
})

describe('hierarchy and materials', () => {
  const node = (name: string, parent: number, parts: Partial<LunaNodeSpec> = {}): LunaNodeSpec => ({ ...ROOT, name, parent, ...parts })

  it('keeps the nodes, their parents and their transforms', () => {
    const nodes = [node('A', -1, { position: [1, 2, 3] }), node('B', 0, { scale: [2, 2, 2], active: false }), node('C', 1, { rotation: [0, 0.7071068, 0, 0.7071068] })]
    const out = buildLunaEffect({ nodes, emitters: [{ emitter: emitter(), node: 2 }], linear: false })
    expect(out.nodes).toEqual(nodes)
    expect(out.emitters[0].node).toBe(2)
  })

  it('copies the nodes instead of sharing them with the caller', () => {
    const nodes = [node('A', -1)]
    const out = buildLunaEffect({ nodes, emitters: [], linear: false })
    expect(out.nodes[0]).not.toBe(nodes[0])
    expect(out.nodes[0].position).not.toBe(nodes[0].position)
  })

  it('rejects a node that comes before its parent', () => {
    expect(() => buildLunaEffect({ nodes: [node('A', 1), node('B', -1)], emitters: [], linear: false })).toThrow(/after its parent/)
    expect(() => buildLunaEffect({ nodes: [node('A', 0)], emitters: [], linear: false })).toThrow(/after its parent/)
  })

  it('rejects an emitter that names a node outside the list', () => {
    expect(() => buildLunaEffect({ nodes: [ROOT], emitters: [{ emitter: emitter(), node: 1 }], linear: false })).toThrow(/names node 1/)
    expect(() => buildLunaEffect({ nodes: [ROOT], emitters: [{ emitter: emitter(), node: -1 }], linear: false })).toThrow(/names node -1/)
  })

  it('shares one material between emitters that hand over the same spec', () => {
    const a = mapMaterial(undefined)
    const b = { ...a }
    const list: BridgeEmitter[] = [
      { emitter: emitter(), node: 0, material: a },
      { emitter: emitter(), node: 0, material: b },
      { emitter: emitter(), node: 0, material: a }
    ]
    const out = buildLunaEffect({ nodes: [ROOT], emitters: list, linear: false })
    expect(out.materials).toHaveLength(2)
    expect(out.emitters.map((e) => e.material)).toEqual([0, 1, 0])
  })

  it('draws a system without a material with the default particle material', () => {
    const out = buildLunaEffect({ nodes: [ROOT], emitters: [{ emitter: emitter(), node: 0 }, { emitter: emitter(), node: 0 }], linear: false })
    expect(out.materials).toHaveLength(1)
    expect(out.materials[0]).toEqual(defaultParticleMaterial())
  })

  it('carries the colour space of the project', () => {
    expect(buildLunaEffect({ nodes: [ROOT], emitters: [], linear: true }).linear).toBe(true)
  })

  it('produces a value the host can clone as a message', () => {
    const colour: MinMaxColor = { ...WHITE, mode: 1, gradient: { colors: [{ t: 0, r: 1, g: 1, b: 1 }], alphas: [{ t: 0, a: 1 }] } }
    const out = buildLunaEffect({ nodes: [ROOT], emitters: [{ emitter: emitter({}, { startColor: colour }), node: 0 }], linear: false })
    expect(structuredClone(out)).toEqual(out)
  })
})

describe('nodesOf (hierarchy from three.js objects)', () => {
  const obj = (name: string, parent: TransformLike | null, parts: Partial<TransformLike> = {}): TransformLike => ({
    name,
    parent,
    position: { x: 0, y: 0, z: 0 },
    quaternion: { x: 0, y: 0, z: 0, w: 1 },
    scale: { x: 1, y: 1, z: 1 },
    visible: true,
    ...parts
  })

  it('lists parents before children and leaves the root out', () => {
    const root = obj('Group', null)
    const a = obj('A', root)
    const b = obj('B', a)
    const c = obj('C', root)
    const { nodes, indexOf } = nodesOf([b, c, a], root)
    expect(nodes.map((n) => [n.name, n.parent])).toEqual([['A', -1], ['B', 0], ['C', -1]])
    expect(indexOf.get(b)).toBe(1)
    expect(indexOf.get(root)).toBeUndefined()
  })

  it('undoes the mirror along X that the prefab build applies', () => {
    const root = obj('Group', null)
    const a = obj('A', root, { position: { x: -1, y: 2, z: 3 }, quaternion: { x: 0.1, y: -0.2, z: -0.3, w: 0.9 }, scale: { x: 2, y: 3, z: 4 } })
    const [n] = nodesOf([a], root).nodes
    expect(n.position).toEqual([1, 2, 3])
    expect(n.rotation).toEqual([0.1, 0.2, 0.3, 0.9])
    expect(n.scale).toEqual([2, 3, 4])
  })

  it('keeps the visibility as the active flag', () => {
    const root = obj('Group', null)
    expect(nodesOf([obj('A', root, { visible: false })], root).nodes[0].active).toBe(false)
  })

  it('takes a node once when two systems share it', () => {
    const root = obj('Group', null)
    const a = obj('A', root)
    expect(nodesOf([a, a], root).nodes).toHaveLength(1)
  })

  it('treats an object outside the root as a root of its own', () => {
    const root = obj('Group', null)
    const stray = obj('Stray', obj('Elsewhere', null))
    expect(nodesOf([stray], root).nodes.map((n) => [n.name, n.parent])).toEqual([['Elsewhere', -1], ['Stray', 0]])
  })

  it('stops at a hierarchy that is far too deep', () => {
    const root = obj('Group', null)
    let leaf = root
    for (let i = 0; i < 100; i++) leaf = obj(`L${i}`, leaf)
    expect(() => nodesOf([leaf], root)).toThrow(/deeper than/)
  })
})
