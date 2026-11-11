import { describe, expect, it } from 'vitest'
import * as THREE from 'three'
import type { ParticleEmitter, ParticleSim } from '@shared/types'
import { EffectPlayer, EmitterPlayer } from './particlePlayer'

const k = (scalar: number) => ({ mode: 0 as const, scalar, min: scalar })
const white = { mode: 0 as const, max: [1, 1, 1, 1] as [number, number, number, number], min: [1, 1, 1, 1] as [number, number, number, number] }

function sim(over: Partial<ParticleSim>): ParticleSim {
  return {
    duration: 1,
    looping: false,
    prewarm: false,
    worldSpace: false,
    simulationSpeed: 1,
    startDelay: k(0),
    startLifetime: k(0.1),
    startSpeed: k(2),
    startSize: k(1),
    startRotation: k(0),
    startColor: white,
    gravity: k(0),
    maxParticles: 100,
    rate: k(0),
    bursts: [],
    lengthScale: 2,
    velocityScale: 0,
    pivot: [0, 0, 0],
    sortingOrder: 0,
    sortingFudge: 0,
    maxScreenSize: 0.5,
    alignment: 'view',
    ...over
  }
}

const emitter = (id: string, s: ParticleSim): ParticleEmitter => ({
  id,
  name: id,
  duration: s.duration,
  looping: s.looping,
  prewarm: false,
  lifetime: 0.1,
  speed: 2,
  size: 1,
  maxParticles: 100,
  rate: 0,
  bursts: 5,
  peak: 5,
  renderMode: 'billboard',
  modules: [],
  subEmitters: 0,
  sim: s
})

describe('particle player', () => {
  it('emits bursts, starts death sub emitters where particles die, and draws trails', () => {
    const root = new THREE.Group()
    const parent = new EmitterPlayer(
      emitter('1', sim({ bursts: [{ time: 0, count: k(5), cycles: 1, interval: 0.01, probability: 1 }], subEmitters: [{ id: '2', type: 'death', inherit: 1, probability: 1 }], trail: { ratio: 1, lifetime: k(1), minVertexDistance: 0, sizeAffectsWidth: false, inheritParticleColor: false, width: k(0.1), colorOverLifetime: white, colorOverTrail: white } })),
      { blend: 'additive', tint: [1, 1, 1, 1] }
    )
    const child = new EmitterPlayer(emitter('2', sim({ startLifetime: k(1), bursts: [{ time: 0, count: k(3), cycles: 1, interval: 0.01, probability: 1 }] })), { blend: 'alpha', tint: [1, 1, 1, 1] })
    root.add(parent.mesh, child.mesh)
    child.isSub = true
    child.restart()
    parent.subs.push({ player: child, type: 'death', inherit: 1, probability: 1 })
    const effect = new EffectPlayer([parent, child])

    effect.update(1 / 30)
    expect(parent.alive).toBe(5)
    // A sub emitter waits for its parent: it emits nothing on its own.
    expect(child.alive).toBe(0)
    // Particles live 0.1 s (3 frames): the trail is checked while they are still alive.
    effect.update(1 / 30)
    const trail = parent.mesh.children[1] as THREE.Mesh
    expect((trail.geometry as THREE.InstancedBufferGeometry).instanceCount).toBeGreaterThan(0)
    for (let i = 0; i < 4; i++) effect.update(1 / 30)
    // 5 particles died: 3 particles each from the sub emitter.
    expect(parent.alive).toBe(0)
    expect(child.alive).toBe(15)
    expect(effect.emitters.every((e) => e.finished)).toBe(false)
  })
})

describe('camera framing', () => {
  it('frames where particles really go, tighter than the worst-case reach, and starts the effect over', () => {
    const root = new THREE.Group()
    const s = sim({ startLifetime: k(1), startSpeed: k(2), startSize: k(0.2), bursts: [{ time: 0, count: k(60), cycles: 1, interval: 0.01, probability: 1 }] })
    const player = new EmitterPlayer({ ...emitter('1', s), lifetime: 1 }, { blend: 'additive', tint: [1, 1, 1, 1] })
    const holder = new THREE.Object3D()
    root.add(holder)
    holder.add(player.mesh)
    const effect = new EffectPlayer([player])
    const worst = effect.extent().getSize(new THREE.Vector3())
    const measured = effect.measuredExtent()
    expect(measured.isEmpty()).toBe(false)
    const size = measured.getSize(new THREE.Vector3())
    expect(size.x).toBeLessThanOrEqual(worst.x + 1e-6)
    expect(size.y).toBeLessThanOrEqual(worst.y + 1e-6)
    expect(size.z).toBeLessThanOrEqual(worst.z + 1e-6)
    expect(size.x * size.y * size.z).toBeLessThan(worst.x * worst.y * worst.z)
    expect(effect.alive).toBe(0)
  })

  it('falls back on the worst-case reach when nothing is emitted', () => {
    const root = new THREE.Group()
    const player = new EmitterPlayer(emitter('1', sim({})), { blend: 'alpha', tint: [1, 1, 1, 1] })
    root.add(player.mesh)
    const effect = new EffectPlayer([player])
    expect(effect.measuredExtent().equals(effect.extent())).toBe(true)
  })
})

describe('firing shots on top (spam like Unity)', () => {
  const burst = (over: Partial<ParticleSim> = {}) => sim({ startLifetime: k(0.5), bursts: [{ time: 0, count: k(10), cycles: 1, interval: 0.01, probability: 1 }], ...over })
  const make = (s: ParticleSim) => {
    const root = new THREE.Group()
    const player = new EmitterPlayer({ ...emitter('1', s), lifetime: 0.5 }, { blend: 'additive', tint: [1, 1, 1, 1] })
    root.add(player.mesh)
    return { root, effect: new EffectPlayer([player]) }
  }

  it('adds a play next to the running one, removed once it is over', () => {
    const { root, effect } = make(burst())
    effect.loop = false
    effect.update(0.1)
    expect(effect.alive).toBe(10)
    effect.fire()
    effect.update(0.1)
    // The shot plays at the same place, alongside the first play.
    expect(root.children).toHaveLength(2)
    effect.update(2)
    effect.update(0.1)
    expect(root.children).toHaveLength(1)
  })

  it('plays one cycle of a looping system, and is cleared by a restart', () => {
    const { root, effect } = make(burst({ looping: true, duration: 0.5 }))
    effect.fire()
    effect.update(0.1)
    expect(root.children).toHaveLength(2)
    for (let i = 0; i < 20; i++) effect.update(0.1)
    // The original keeps looping; the shot is gone after its one cycle.
    expect(root.children).toHaveLength(1)
    effect.fire()
    effect.restart()
    expect(root.children).toHaveLength(1)
  })

  it('keeps at most a dozen shots playing', () => {
    const { root, effect } = make(burst({ startLifetime: k(10) }))
    for (let i = 0; i < 20; i++) effect.fire()
    expect(root.children).toHaveLength(13)
  })
})


describe('timeline', () => {
  it('shows the same particles when going back and forth on it', () => {
    const root = new THREE.Group()
    const s = sim({ startLifetime: k(1), startSpeed: { mode: 3 as const, scalar: 5, min: 1 }, rate: k(30), duration: 1 })
    const player = new EmitterPlayer({ ...emitter('1', s), lifetime: 1 }, { blend: 'alpha', tint: [1, 1, 1, 1] })
    root.add(player.mesh)
    const effect = new EffectPlayer([player])
    const at = () => {
      const points: number[] = []
      player.sample((p) => points.push(p.x, p.y, p.z))
      return points
    }
    effect.seek(0.5)
    const first = at()
    expect(first.length).toBeGreaterThan(0)
    expect(effect.time).toBeCloseTo(0.5)
    effect.seek(0.2)
    expect(effect.time).toBeCloseTo(0.2)
    effect.seek(0.5)
    expect(at()).toEqual(first)
    expect(effect.timeline()).toBe(2)
  })
})
