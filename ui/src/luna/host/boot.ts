/**
 * Starts the Luna runtime inside the host page. This is the ONLY module that calls Luna internals, so a Luna version
 * bump has one place to check. The Luna scripts are minified and have no typings, so values from them are `Dyn`.
 *
 * Luna internal names this module depends on (all checked on Luna 6.3 to 7.2; none is a minified name):
 *   Globals the scripts read before they load: MODULE_reflection, MODULE_prefabs, DEBUG, TRACE, DEVELOP, TESTS,
 *     FORCE_STABLE_RANDOM_SEED, $environment, Deserializers, ShaderProgramsLibrary, pi.logEvent, pi.logError.
 *   LunaUnity.Application (+ .StartupScene), .InitializeAsync, .LoadProjectSettingsAsync, .app (the pc.Application),
 *     .FireRuntimeInitializationCallbacks
 *   pc.ProjectSettings.instance (physicsSettings, physics2DSettings, desiredColorSpace, allTagsInProject, timeSettings
 *     with fixedDeltaTime / miximumDeltaTime / timeScale / maximumParticleTimestep)
 *   pc.Application: mimicUnityMouse, initializeBuiltInTextures, setCanvasFillMode, setCanvasResolution,
 *     initializeRenderer, start, graphicsDevice
 *   LunaUnity.Objects.Bundle.configure, LunaUnity.Objects.Scene (initializeAsTemplate, createInstance, _load,
 *     _installTransforms, projectSettings)
 *   UnityEngine.RenderSettings.ambientProbe, UnityEngine.Debug.LogError$1 (the engine reports shader compile errors there)
 *   pc.UnityShaderPass, pc.UnityShader (setup, compileVariants, $id, guid, passes, usePasses, shaderDefinedKeywords,
 *     defaultParameterValues), UnityEngine.Shader.shaderIndex (entries are added and removed by name and by $id), ShaderProgramsLibrary (`<name>_vs` / `_fs`
 *     entries are removed again), pass.webgl2Variants
 *   Bridge overloads: GameObject.$ctor2(name), Material.$ctor2(shader), Texture2D.$ctor1(w, h),
 *     Material.SetColor$1 / SetFloat$1 / SetTexture$1 / SetVector$1 (property-name overloads), Material.renderQueue
 *   pc.UnityShaderPass state fields: zTest, zWrite, culling, colorWriteMask, blending, alphaBlending (Unity enums in boxes)
 *   Texture2D.handle (a pc.Texture): setSource, lock, unlock, _flipY, addressU, addressV, minFilter, magFilter,
 *     mipmaps, sRGBTexture; constants pc.ADDRESS_* and pc.FILTER_*. See `createTexture`.
 *   pc.GraphicsDevice.setTexture(handle, unit) (uploads a texture that has new pixels), .gl (the WebGL2 context)
 *   Colour space: Deserializers.linearColorSpace (read when InitializeAsync creates the graphics device, so it is a boot
 *     choice) and pc.ProjectSettings.instance.desiredColorSpace (0 gamma, 1 linear)
 *   Hierarchy (`apply.ts`): GameObject.$ctor2, Transform.parent / localPosition / localRotation / localScale,
 *     GameObject.SetActive, activeInHierarchy, AddComponent, GetComponent
 *   ParticleSystem: Play$1 / Pause$1 (with-children flag), Stop$2, Simulate(time, withChildren, restart), IsAlive, isPlaying,
 *     particleCount, time. Modules main, emission, shape, velocityOverLifetime, limitVelocityOverLifetime,
 *     inheritVelocity, forceOverLifetime, colorOverLifetime, colorBySpeed, sizeOverLifetime, sizeBySpeed,
 *     rotationOverLifetime, rotationBySpeed, noise, textureSheetAnimation, each with its Unity property names.
 *     Module curve properties return the live MinMaxCurve (mode, constant, constantMin, constantMax, curve, curveMin,
 *     curveMax, curveMultiplier, evaluate) and gradient properties the live MinMaxGradient (internalMode, _colorMax,
 *     _colorMin, gradient, gradientMin, evaluate). Neither has a setter for the whole object, so apply.ts writes them in place.
 *   pc.AnimationCurve({ keys: [[time, value, inTangent, outTangent, tangentMode]] }), pc.ColorGradient(colorKeys, alphaKeys,
 *     mode) in its flat form (five numbers per colour key, two per alpha key; the property-by-property form does not work)
 *   pc.ParticleSystemBurst (time, minCount, maxCount, cycleCount, repeatInterval, count), emission.setBursts
 *   shape: the `shapeType` setter builds the implementation and resets the fields, so it is set first
 *   ParticleSystemRenderer: sharedMaterial, renderMode, alignment, sortMode, sortingOrder, lengthScale, velocityScale,
 *     cameraVelocityScale, normalDirection, sortingFudge, minParticleSize, maxParticleSize, pivot, applyActiveColorSpace, enabled
 *   ParticleSystem emission: see `setEmissionRate` (module `_rateOverTime`).
 */
import { LUNA_BASE_URL } from '../runtime-info'
import type { LunaSampler, LunaTextureSpec } from '../protocol'
import type { ParticleShaderDef } from './shaders/common'

/** Luna ships no typings. Every value that comes out of its scripts is `Dyn`, and one wrapper function owns each use. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type Dyn = any

/** Scripts in load order. The order follows `engine/luna/manifest.json` priorities with the stub variants. */
const SCRIPTS = [
  'pipeline/templates/html/resources/js/pi.js',
  'engine/luna/script-1.js',
  'pipeline/templates/LunaCompiler/bin/bridge.js',
  'engine/luna/script1.js',
  'engine/luna/particle-system-1.js',
  'engine/luna/urp-1-stub.js',
  'engine/luna/physics3d-0-stub.js',
  'engine/luna/physics3d-1-stub.js',
  'engine/luna/physics2d-1-stub.js',
  'pipeline/templates/LunaCompiler/bin/Bridge.Locales.js',
  'pipeline/templates/LunaCompiler/bin/UnityEngine.js',
  'pipeline/templates/LunaCompiler/bin/UnityEngine.UI.js',
  'pipeline/templates/LunaCompiler/bin/UnityEngine.UniversalRenderPipeline.js',
  'pipeline/templates/LunaCompiler/bin/TextMeshPro.js',
  // script3.js is loaded last, after the stubs below.
  'engine/luna/script3.js'
]

const w = window as unknown as Record<string, Dyn>

export interface Engine {
  /** The LunaUnity.Application. */
  app: Dyn
  /** The Luna scene, which is also `window.$scene`. */
  scene: Dyn
  canvas: HTMLCanvasElement
}

function loadScript(src: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const el = document.createElement('script')
    el.src = src
    el.async = false
    el.onload = () => resolve()
    el.onerror = () => reject(new Error(`Could not load ${src}`))
    document.head.appendChild(el)
  })
}

/** Defines the globals the scripts expect, then loads the scripts one after another. */
export async function loadRuntime(): Promise<void> {
  Object.assign(w, {
    MODULE_reflection: true,
    MODULE_prefabs: true,
    DEBUG: false,
    TRACE: false,
    DEVELOP: true,
    TESTS: false,
    FORCE_STABLE_RANDOM_SEED: false,
    $environment: {
      baseUrl: './',
      resourceConfig: { json: 'external', image: 'external', video: 'external', blob: 'external', sound: 'external' },
      packageConfig: { userId: '', version: '0' },
      playerPrefs: false,
      forceIncludedClasses: [],
      targetPlatform: 'develop',
      runtimeAnalysisModules: []
    },
    // The scripts read the build's type tables. There is no build, so every field index is 0 and no list has entries.
    Deserializers: {
      buildID: 'asset-hub',
      types: [],
      typeNameToIdMap: {},
      requiredComponents: {},
      packagesInfo: '',
      runtimeInitializeOnLoadInfos: new Proxy({}, { get: () => [] }),
      fields: new Proxy({}, { get: () => new Proxy({}, { get: (_t, k) => (typeof k === 'string' ? 0 : undefined) }) })
    },
    pi: w.pi ?? {}
  })
  for (const script of SCRIPTS) {
    // The analytics hooks must exist before script3.js, which is the last script.
    if (script === 'engine/luna/script3.js') {
      w.pi.logEvent ??= () => {}
      w.pi.logError ??= () => {}
    }
    await loadScript(LUNA_BASE_URL + script)
  }
  if (typeof w.pc?.Application !== 'function' || typeof w.UnityEngine?.ParticleSystem !== 'function') {
    throw new Error('The Luna runtime loaded without its particle system')
  }
  forwardEngineErrors()
}

/** The engine reports shader compile and link errors through `Debug.LogError`, which prints nothing here. Send them to the console. */
function forwardEngineErrors(): void {
  const debug = w.UnityEngine.Debug
  const original = debug.LogError$1
  if (typeof original !== 'function') return
  debug.LogError$1 = (message: unknown, ...rest: unknown[]) => {
    console.error('[luna]', String(message))
    return original.call(debug, message, ...rest)
  }
}

function defaultRenderSettings(): Dyn {
  const C = (r: number, g: number, b: number) => new w.pc.Color(r, g, b, 1)
  const probe = w.UnityEngine.RenderSettings.ambientProbe
  return {
    ambientLight: C(0.212, 0.227, 0.259), ambientMode: 0, pixelLightCount: 4,
    ambientSkyColor: C(0.212, 0.227, 0.259), ambientEquatorColor: C(0.114, 0.125, 0.133), ambientGroundColor: C(0.047, 0.043, 0.035),
    ambientIntensity: 1, reflectionIntensity: 1, defaultReflectionMode: 0, defaultReflectionResolution: 128,
    sun: null, skybox: null, mixedBakeMode: 0, fogDensity: 0.01, fogColor: C(0.5, 0.5, 0.5), fogStartDistance: 0, fogEndDistance: 300,
    fogMode: 3, fog: false, hasLightDataAsset: false, hasManualGenerate: false, customReflection: null,
    ambientProbe: probe && probe.clone ? probe.clone() : probe, lightProbes: null, lightmaps: [], lightmapsMode: 0,
    environmentLightingMode: 0, sunLightObjectId: 0, defaultReflection: null
  }
}

/**
 * Starts the application and an empty manual scene on the canvas. `linear` picks the colour space of the whole run: the
 * graphics device reads it once, while it is made, so it cannot change later.
 */
export async function bootEngine(canvas: HTMLCanvasElement, linear: boolean): Promise<Engine> {
  const { pc, LunaUnity, UnityEngine, System } = w
  const app = new LunaUnity.Application(canvas, w.$environment, new LunaUnity.Application.StartupScene('-1', 'Preview'))

  const settings = pc.ProjectSettings.instance
  settings.physicsSettings.gravity = new pc.Vec3(0, -9.81, 0)
  settings.physics2DSettings.gravity = new pc.Vec2(0, -9.81)
  settings.desiredColorSpace = linear ? 1 : 0
  w.Deserializers.linearColorSpace = linear
  settings.allTagsInProject = ['Untagged', 'MainCamera']
  // A zero maximumParticleTimestep makes the particle update loop run forever, so every value is non-zero.
  Object.assign(settings.timeSettings, { fixedDeltaTime: 0.02, miximumDeltaTime: 0.3333333, timeScale: 1, maximumParticleTimestep: 0.03 })
  // A build reads these from a serialized blob. There is none here.
  app.LoadProjectSettingsAsync = () => System.Threading.Tasks.Task.fromResult(true)

  await new Promise<void>((resolve, reject) =>
    app.InitializeAsync().continueWith((t: Dyn) => (t.exception ? reject(new Error(String(t.exception))) : resolve()))
  )
  const pcApp = app.app
  pcApp.mimicUnityMouse()
  pcApp.initializeBuiltInTextures()
  pcApp.setCanvasFillMode(pc.FILLMODE_FILL_WINDOW)
  pcApp.setCanvasResolution(pc.RESOLUTION_AUTO)
  app.FireRuntimeInitializationCallbacks()
  w.ShaderProgramsLibrary ??= {}
  pcApp.initializeRenderer()

  LunaUnity.Objects.Bundle.configure(app, {})
  const template = new LunaUnity.Objects.Scene().initializeAsTemplate({
    renderSettings: defaultRenderSettings(),
    rootJsons: [],
    name: 'Preview',
    index: 0,
    shaders: []
  })
  const scene = template.createInstance()
  scene.projectSettings = pc.ProjectSettings.instance
  scene._installTransforms = () => {}
  scene._load(UnityEngine.SceneManagement.LoadSceneMode.Single)
  if (w.$scene !== scene) throw new Error('The Luna scene did not become active')
  pcApp.start()
  return { app, scene, canvas }
}

/** Frame statistics of the last frame. */
export function drawCallsOfLastFrame(engine: Engine): number {
  const device = engine.app.app.graphicsDevice
  return Number(device._drawCallsPerFrame ?? device.drawCallsPerFrame ?? 0)
}

/** Unity serialises shader state as `{ $$fullname, val }` boxes. */
const box = (val: number): Dyn => ({ $$fullname: 'asset-hub', val })

/**
 * Compiles a shader written in Luna's format and registers it so `Shader.Find(def.name)` returns it.
 * Luna compiles a shader only through `compileVariants()`, which reads `pass.webgl2Variants`. `def.name` must be unique
 * per compiled variant.
 */
export function registerShader(engine: Engine, def: ParticleShaderDef): Dyn {
  const { pc, UnityEngine } = w
  const key = def.name.replace(/\W+/g, '_')
  w.ShaderProgramsLibrary[`${key}_vs`] = def.vertex
  w.ShaderProgramsLibrary[`${key}_fs`] = def.fragment
  const blend = { src: box(def.blend.src), dst: box(def.blend.dst), op: box(0) }
  const stencil = { comp: box(8), pass: box(0), fail: box(0), zFail: box(0) }
  const pass = new pc.UnityShaderPass()
  Object.assign(pass, {
    id: 0, subShaderIndex: 0, name: 'FORWARD', passType: 0,
    tags: [{ name: 'RenderType', value: 'Transparent' }],
    passDefinedKeywords: [], passDefinedKeywordGroups: [],
    zTest: box(4), zWrite: box(def.zWrite ? 1 : 0), culling: box(def.cull), colorWriteMask: box(def.colorMask), offsetUnits: box(0), offsetFactor: box(0),
    blending: blend, alphaBlending: blend,
    stencilRef: box(0), stencilReadMask: box(255), stencilWriteMask: box(255),
    stencilOp: stencil, stencilOpFront: stencil, stencilOpBack: stencil,
    variants: [{ keywords: [], vertexProgram: `${key}_vs`, fragmentProgram: `${key}_fs`, readDepth: false, exportedForWebGl2: true, subShaderIndex: 0, passId: 0 }],
    excludedVariants: []
  })
  const shader = new pc.UnityShader(engine.app.app.graphicsDevice)
  Object.assign(shader, { name: def.name, guid: `asset-hub-${key}`, shaderDefinedKeywords: [], passes: [pass], usePasses: [], defaultParameterValues: [] })
  shader.setup()
  if (!pass.webgl2Variants || pass.webgl2Variants.length === 0) pass.webgl2Variants = pass.variants
  shader.compileVariants()
  UnityEngine.Shader.shaderIndex[shader.$id] = shader
  UnityEngine.Shader.shaderIndex[def.name] = shader
  return UnityEngine.Shader.Find(def.name)
}

/** Forgets a shader made by `registerShader`: its program sources and its entries in the shader index. */
export function unregisterShader(shader: Dyn, name: string): void {
  const { UnityEngine } = w
  const key = name.replace(/\W+/g, '_')
  delete w.ShaderProgramsLibrary[`${key}_vs`]
  delete w.ShaderProgramsLibrary[`${key}_fs`]
  delete UnityEngine.Shader.shaderIndex[shader.$id]
  delete UnityEngine.Shader.shaderIndex[name]
}

/** The sampler fields applied to a new texture. Unity's `FilterMode` has three values, the engine has six. */
function applySampler(handle: Dyn, sampler: LunaSampler): void {
  const { pc } = w
  const address = { repeat: pc.ADDRESS_REPEAT, clamp: pc.ADDRESS_CLAMP_TO_EDGE, mirror: pc.ADDRESS_MIRRORED_REPEAT }
  handle.addressU = address[sampler.wrapU]
  handle.addressV = address[sampler.wrapV]
  handle.mipmaps = sampler.mipmaps
  handle.sRGBTexture = sampler.srgb
  if (sampler.filter === 'point') {
    handle.minFilter = sampler.mipmaps ? pc.FILTER_NEAREST_MIPMAP_NEAREST : pc.FILTER_NEAREST
    handle.magFilter = pc.FILTER_NEAREST
  } else {
    const trilinear = sampler.filter === 'trilinear'
    handle.minFilter = !sampler.mipmaps ? pc.FILTER_LINEAR : trilinear ? pc.FILTER_LINEAR_MIPMAP_LINEAR : pc.FILTER_LINEAR_MIPMAP_NEAREST
    handle.magFilter = pc.FILTER_LINEAR
  }
}

/**
 * Makes an engine texture from pixels, uploading straight to the GPU texture object (`Texture2D.handle`, a
 * pc.Texture). `SetPixel` per pixel is far too slow for real sizes, `SetPixels` writes nothing, and
 * `Texture2D.whiteTexture` samples black, so this is also how the 1x1 white fallback is made.
 * Unity stores the bottom row first. Image data has the top row first, so both paths flip it.
 * The sampler fields are set before the pixels, because the engine reads them when it uploads.
 */
export function createTexture(source: LunaTextureSpec): Dyn {
  const { UnityEngine } = w
  const width = 'bitmap' in source ? source.bitmap.width : source.width
  const height = 'bitmap' in source ? source.bitmap.height : source.height
  if (!(width > 0 && height > 0)) throw new Error('A texture needs a positive size')
  const texture = new UnityEngine.Texture2D.$ctor1(width, height)
  const handle: Dyn = texture.handle
  applySampler(handle, source)
  if ('bitmap' in source) {
    // WebGL ignores the flip for an ImageBitmap, so the bitmap goes through a canvas, which honours `_flipY`.
    handle._flipY = true
    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    const context = canvas.getContext('2d')
    if (!context) throw new Error('No 2D canvas to prepare the texture')
    context.drawImage(source.bitmap, 0, 0)
    handle.setSource(canvas)
  } else {
    if (source.rgba.length !== width * height * 4) throw new Error('Texture bytes do not match its size')
    // The flip flag does not apply to raw bytes, so the rows are reversed here.
    const row = width * 4
    const pixels: Uint8Array = handle.lock()
    for (let y = 0; y < height; y++) pixels.set(source.rgba.subarray(y * row, (y + 1) * row), (height - 1 - y) * row)
    handle.unlock()
  }
  return texture
}

/**
 * Uploads a texture to the GPU now, instead of at its first draw, which would hitch the first frame. `setTexture` is
 * what the draw call itself uses, so it binds the texture and uploads it with its mipmaps if it has new pixels.
 */
export function uploadNow(engine: Engine, texture: Dyn): void {
  engine.app.app.graphicsDevice.setTexture(texture.handle, 0)
}

/** Waits until the GPU has finished the commands so far, so a timing around uploads covers the real work. */
export function finishGpu(engine: Engine): void {
  engine.app.app.graphicsDevice.gl.finish()
}
