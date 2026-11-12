/** Builds a particle effect from a `LunaEffectSpec` and drives it. Uses Luna's `UnityEngine` API through `Dyn` values. */
import type { LunaCamera, LunaEffectSpec, LunaStats, Rgba } from '../protocol'
import { activateNodes, createNodes, createSystem } from './apply'
import { drawCallsOfLastFrame, type Dyn, type Engine } from './boot'
import { MaterialFactory } from './material'

const w = window as unknown as Record<string, Dyn>

/** Shots fired on top of the effect at once at most. */
const MAX_SHOTS = 4

/** Reports something the preview could not reproduce. `emitter` names the particle system it concerns. */
export type EmitterReport = (what: string, detail: string, emitter?: string) => void

/** One copy of the prefab: its GameObjects and particle systems. */
interface Instance {
  nodes: Dyn[]
  systems: Dyn[]
  /** The systems that start on their own, which are the ones with Play On Awake. */
  awake: Dyn[]
}

export class EffectPreview {
  private readonly factory: MaterialFactory
  private spec: LunaEffectSpec | null = null
  private materials: Dyn[] = []
  private base: Instance | null = null
  private shots: Instance[] = []
  private readonly camera: Dyn
  private readonly cameraObject: Dyn
  private speed = 1
  /** A finished effect starts over by itself. */
  private loop = true
  /** Held by a pause or a seek: not finished, so it does not start over. */
  private held = false

  /** `linear` is the colour space the engine was started in. An effect of the other space is refused. */
  constructor(
    private readonly engine: Engine,
    private readonly report: EmitterReport,
    private readonly linear: boolean
  ) {
    const { UnityEngine, pc } = w
    this.factory = new MaterialFactory(engine, this.report)
    const go = new UnityEngine.GameObject.$ctor2('Camera')
    const camera = go.AddComponent(UnityEngine.Camera)
    // Until the app frames the effect: prefab effects are a few units across and sit at the origin. The camera looks along +Z, with the origin a little below its centre.
    go.transform.position = new pc.Vec3(0, 0.6, -4.5)
    camera.clearFlags = UnityEngine.CameraClearFlags.SolidColor
    camera.backgroundColor = new pc.Color(0.1, 0.1, 0.2, 1)
    this.camera = camera
    this.cameraObject = go
  }

  /** Puts the camera where the app says, so the effect is framed as the Unity-style view frames it. */
  setCamera(view: LunaCamera): void {
    const { pc } = w
    const t = this.cameraObject.transform
    t.position = new pc.Vec3(view.position[0], view.position[1], view.position[2])
    t.localRotation = new pc.Quat(view.rotation[0], view.rotation[1], view.rotation[2], view.rotation[3])
    this.camera.fieldOfView = view.fov
    this.camera.nearClipPlane = view.near
    this.camera.farClipPlane = view.far
  }

  setBackground(color: Rgba): void {
    this.camera.backgroundColor = new w.pc.Color(color[0], color[1], color[2], color[3])
  }

  /** Plays everything at `value` times the speed the asset sets, now and for shots fired later. */
  setSpeed(value: number): void {
    this.speed = value
    for (const instance of this.instances()) this.applySpeed(instance)
  }

  setLoop(value: boolean): void {
    this.loop = value
  }

  /** Builds the effect. Returns the milliseconds spent making textures. */
  load(effect: LunaEffectSpec): number {
    this.clear()
    if (effect.linear !== this.linear) {
      throw new Error(`The effect is for ${effect.linear ? 'linear' : 'gamma'} colour space, but the host runs in ${this.linear ? 'linear' : 'gamma'}`)
    }
    for (const e of effect.emitters) {
      if (!Number.isInteger(e.material) || e.material < 0 || e.material >= effect.materials.length) {
        throw new Error(`Emitter ${e.name} names material ${e.material}, but the effect has ${effect.materials.length}`)
      }
      if (!Number.isInteger(e.node) || e.node < 0 || e.node >= effect.nodes.length) {
        throw new Error(`Emitter ${e.name} names node ${e.node}, but the effect has ${effect.nodes.length}`)
      }
    }
    this.spec = effect
    try {
      this.materials = effect.materials.map((m) => this.factory.create(m))
      for (const e of effect.emitters) for (const u of e.unsupported) this.report(u.module, u.detail, e.name)
      this.base = this.instantiate(effect, false)
    } catch (error) {
      // Nothing half-built stays in the scene.
      this.clear()
      throw error
    }
    return this.factory.takeTextureMs()
  }

  play(): void {
    this.held = false
    for (const instance of this.instances()) this.start(instance)
  }

  pause(): void {
    this.held = true
    for (const instance of this.instances()) for (const ps of instance.systems) ps.Pause$1(false)
  }

  restart(): void {
    this.held = false
    this.removeShots()
    if (!this.base) return
    for (const ps of this.base.systems) {
      // Play on a paused system only resumes it, so play first to unpause, then restart from a clean state.
      ps.Play$1(false)
      ps.Stop$2(false, w.UnityEngine.ParticleSystemStopBehavior.StopEmittingAndClear)
    }
    this.start(this.base)
  }

  /** Jumps to `time` seconds after the start and holds there. */
  seek(time: number): void {
    this.held = true
    this.removeShots()
    // Simulate restarts the system, steps it forward and leaves it paused. Each system steps on its own.
    for (const ps of this.base?.awake ?? []) ps.Simulate(Math.max(0, time), false, true)
  }

  /** Plays the effect once more on top of what is running. */
  fire(): void {
    if (!this.spec) return
    if (this.shots.length >= MAX_SHOTS) this.destroy(this.shots.shift() as Instance)
    const shot = this.instantiate(this.spec, true)
    this.shots.push(shot)
  }

  /** Removes finished shots, and reads the state for the `time` event. */
  tick(): LunaStats {
    this.shots = this.shots.filter((shot) => {
      const alive = shot.systems.some((ps) => ps.IsAlive(false) || ps.isPlaying)
      if (!alive) this.destroy(shot)
      return alive
    })
    // With Loop on, an effect whose systems are all done plays again.
    const base = this.base
    if (this.loop && !this.held && base && base.awake.length > 0 && !base.awake.some((ps) => ps.IsAlive(false) || ps.isPlaying)) this.restart()
    let particles = 0
    for (const instance of this.instances()) for (const ps of instance.systems) particles += ps.particleCount
    return { time: this.base?.awake[0]?.time ?? this.base?.systems[0]?.time ?? 0, particles, drawCalls: drawCallsOfLastFrame(this.engine) }
  }

  get loaded(): boolean {
    return this.spec !== null
  }

  /** Removes the effect. The camera stays, so a later `load` works. The app removes the whole iframe at the end. */
  dispose(): void {
    this.clear()
  }

  private instances(): Instance[] {
    return this.base ? [this.base, ...this.shots] : []
  }

  private clear(): void {
    this.removeShots()
    if (this.base) this.destroy(this.base)
    this.base = null
    this.materials = []
    this.factory.release()
    this.spec = null
  }

  private removeShots(): void {
    for (const shot of this.shots) this.destroy(shot)
    this.shots = []
  }

  /** Destroying the root objects removes their children as well. */
  private destroy(instance: Instance): void {
    const spec = this.spec
    instance.nodes.forEach((go, i) => {
      if (!spec || spec.nodes[i].parent < 0) w.UnityEngine.Object.DestroyImmediate(go)
    })
  }

  /** Starts the systems that play on awake. A system on an inactive node waits until its node is active. */
  private start(instance: Instance): void {
    for (const ps of instance.awake) if (ps.gameObject.activeInHierarchy) ps.Play$1(false)
  }

  private instantiate(spec: LunaEffectSpec, shot: boolean): Instance {
    const nodes: Dyn[] = []
    let systems: Dyn[]
    try {
      createNodes(spec.nodes, nodes)
      systems = spec.emitters.map((e) => createSystem(nodes[e.node], e, this.materials[e.material], shot ? false : e.main.loop))
      activateNodes(spec.nodes, nodes)
    } catch (error) {
      // The roots are enough: destroying a root removes its children.
      this.destroy({ nodes, systems: [], awake: [] })
      throw error
    }
    const instance: Instance = { nodes, systems, awake: systems.filter((_, i) => spec.emitters[i].main.playOnAwake) }
    this.applySpeed(instance)
    this.start(instance)
    return instance
  }

  private applySpeed(instance: Instance): void {
    const spec = this.spec
    if (!spec) return
    instance.systems.forEach((ps, i) => {
      ps.main.simulationSpeed = spec.emitters[i].main.simulationSpeed * this.speed
    })
  }
}
