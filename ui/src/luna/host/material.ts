/**
 * Builds Luna materials from `LunaMaterialSpec`s: picks and compiles the shader module, uploads the textures and sets
 * the properties. A material whose shader has no module is drawn with the fallback shader and reported as `unsupported`.
 */
import type { LunaMaterialSpec, LunaTextureSpec } from '../protocol'
import { createTexture, finishGpu, registerShader, unregisterShader, uploadNow, type Dyn, type Engine } from './boot'
import { FALLBACK_SHADER, SHADER_MODULES } from './shaders'

const w = window as unknown as Record<string, Dyn>

const WHITE: LunaTextureSpec = {
  width: 1,
  height: 1,
  rgba: new Uint8ClampedArray([255, 255, 255, 255]),
  wrapU: 'clamp',
  wrapV: 'clamp',
  filter: 'point',
  srgb: false,
  mipmaps: false
}

const SAMPLER_KEYS = ['wrapU', 'wrapV', 'filter', 'srgb', 'mipmaps'] as const

/** Small stable hash of a string, for shader variant names. */
function hash(text: string): string {
  let h = 5381
  for (let i = 0; i < text.length; i++) h = ((h << 5) + h + text.charCodeAt(i)) | 0
  return (h >>> 0).toString(16)
}

export type Report = (what: string, detail: string) => void

export class MaterialFactory {
  /** Compiled shaders by variant key. `used` is the effect (counted by `release`) that last asked for it. */
  private readonly shaders = new Map<string, { shader: Dyn; queue: number; name: string; used: number }>()
  /** Effects released so far. */
  private generation = 0
  /** Textures made for the current effect. The same source with the same sampler is uploaded once. */
  private readonly textures = new Map<object, Map<string, Dyn>>()
  private white: Dyn = null
  private materials: Dyn[] = []
  private reported = new Set<string>()
  /** Milliseconds spent making textures since the last `takeTextureMs`. */
  private textureMs = 0

  constructor(
    private readonly engine: Engine,
    private readonly report: Report
  ) {}

  /** Makes the material of a spec. Reports what the preview could not reproduce, once per effect. */
  create(spec: LunaMaterialSpec): Dyn {
    const { UnityEngine, pc } = w
    const { shader, queue } = this.shaderOf(spec)
    const material = new UnityEngine.Material.$ctor2(shader)
    material.renderQueue = queue
    for (const [name, value] of Object.entries(spec.floats)) material.SetFloat$1(name, value)
    for (const [name, c] of Object.entries(spec.colors)) material.SetColor$1(name, new pc.Color(c[0], c[1], c[2], c[3]))
    for (const [name, v] of Object.entries(spec.vectors)) material.SetVector$1(name, new pc.Vec4(v[0], v[1], v[2], v[3]))
    for (const [name, texture] of Object.entries(spec.textures)) material.SetTexture$1(name, this.texture(texture))
    for (const note of spec.notes) this.reportOnce('material', `${spec.shaderName}: ${note}`)
    this.materials.push(material)
    return material
  }

  /**
   * Destroys the materials and textures of the current effect. The compiled shaders of this effect stay for the next one,
   * which is often the same prefab again. A shader that neither this effect nor the one before used is forgotten.
   */
  release(): void {
    // An empty release (a dispose after a dispose, a load after a dispose) is not another effect.
    if (this.materials.length > 0) this.generation++
    for (const [key, entry] of this.shaders) {
      if (entry.used >= this.generation - 1) continue
      unregisterShader(entry.shader, entry.name)
      this.shaders.delete(key)
    }
    for (const material of this.materials) w.UnityEngine.Object.DestroyImmediate(material)
    this.materials = []
    for (const bySampler of this.textures.values()) for (const texture of bySampler.values()) w.UnityEngine.Object.DestroyImmediate(texture)
    this.textures.clear()
    if (this.white) w.UnityEngine.Object.DestroyImmediate(this.white)
    this.white = null
    this.reported.clear()
  }

  /** Waits for the GPU to take the uploads. Returns the milliseconds spent making and uploading textures since the last call. */
  takeTextureMs(): number {
    const start = performance.now()
    finishGpu(this.engine)
    const ms = this.textureMs + (performance.now() - start)
    this.textureMs = 0
    return ms
  }

  private reportOnce(what: string, detail: string): void {
    const key = `${what}\n${detail}`
    if (this.reported.has(key)) return
    this.reported.add(key)
    this.report(what, detail)
  }

  private shaderOf(spec: LunaMaterialSpec): { shader: Dyn; queue: number } {
    const known = spec.shader ? SHADER_MODULES[spec.shader] : undefined
    if (!known) this.reportOnce('shader', spec.shaderName)
    const module = known ?? FALLBACK_SHADER
    const options = { keywords: [...spec.keywords].sort(), state: spec.state }
    const key = `${module.id}\n${JSON.stringify(options)}`
    let entry = this.shaders.get(key)
    if (entry) entry.used = this.generation
    else {
      const def = module.build(options)
      // Each variant needs its own name, because the engine finds a shader by name.
      const name = `${def.name} #${hash(key)}`
      const shader = registerShader(this.engine, { ...def, name })
      entry = { shader, queue: def.queue, name, used: this.generation }
      this.shaders.set(key, entry)
    }
    return entry
  }

  private texture(spec: LunaTextureSpec | null): Dyn {
    if (!spec) return (this.white ??= this.make(WHITE))
    const source: object = 'bitmap' in spec ? spec.bitmap : spec.rgba
    const samplerKey = SAMPLER_KEYS.map((k) => String(spec[k])).join('|')
    let bySampler = this.textures.get(source)
    if (!bySampler) this.textures.set(source, (bySampler = new Map()))
    let texture = bySampler.get(samplerKey)
    if (!texture) bySampler.set(samplerKey, (texture = this.make(spec)))
    return texture
  }

  private make(spec: LunaTextureSpec): Dyn {
    const start = performance.now()
    const texture = createTexture(spec)
    uploadNow(this.engine, texture)
    this.textureMs += performance.now() - start
    return texture
  }
}
