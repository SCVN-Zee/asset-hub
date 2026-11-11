import * as THREE from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import { FBXLoader } from 'three/examples/jsm/loaders/FBXLoader.js'
import { TGALoader } from 'three/examples/jsm/loaders/TGALoader.js'
import type { MaterialLook, ModelStats, TextureRole } from '@shared/types'
import { contextRecovery } from './contextRecovery'

export type ViewPreset = 'front' | 'side' | 'top' | 'reset'

/** Shape a material is shown on. */
export type PreviewShape = 'sphere' | 'cube' | 'plane'

/** A texture file fetched from the repo, served to three.js through a blob URL. */
export interface TextureFile {
  name: string
  url: string
  bytes: number
  width?: number
  height?: number
  /** What the file name suggests the texture is for (the Rust side guesses it). */
  role?: TextureRole
}

/** A .mat texture slot to apply to the FBX material with the same name (kind A); '*' applies to every material. */
export interface MaterialSlot {
  material: string
  prop: string
  file: TextureFile
}

/**
 * Stands in for a texture that is not found: plain white, so the material's
 * colour and vertex colours still show. (A transparent one turned untextured
 * models black and see-through: rigs like Customer_Red.fbx vanished.)
 */
export const BLANK_TEXTURE = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC'
/** In priority order: URP's _BaseMap wins over the legacy _MainTex that URP materials also keep. */
const COLOR_ORDER = ['_basemap', '_basecolormap', '_albedo', '_maintex', '_diffuse']
const COLOR_PROPS = new Set(COLOR_ORDER)
const slotRank = (prop: string) => (COLOR_PROPS.has(prop) ? COLOR_ORDER.indexOf(prop) : COLOR_ORDER.length)
const NORMAL_PROPS = new Set(['_bumpmap', '_normalmap'])
const EMISSION_PROPS = new Set(['_emissionmap'])

const baseLower = (url: string) => decodeURIComponent(url).split(/[\\/]/).pop()!.toLowerCase()

/** Neutral greys a little darker than the 18% grey stage: a faint spatial reference in the live viewer. */
const GRID_CENTER = 0x666666
const GRID_LINE = 0x6f6f6f

/** Imperative three.js viewer: orbit/zoom/pan, view presets, wireframe, animations, thumbnails. */
export class FbxViewer {
  private renderer!: THREE.WebGLRenderer
  private readonly scene = new THREE.Scene()
  private readonly camera = new THREE.PerspectiveCamera(45, 1, 0.01, 1e6)
  private controls!: OrbitControls
  private readonly clock = new THREE.Clock()
  private readonly resize: ResizeObserver
  private model?: THREE.Group
  private mixer?: THREE.AnimationMixer
  private grid?: THREE.GridHelper
  private home = { position: new THREE.Vector3(), target: new THREE.Vector3(), radius: 1 }
  private frame = 0
  private disposed = false
  /** A turn of the camera around its target under way (`lookFrom`): from and to directions, and how far along. */
  private turn?: { from: THREE.Vector3; to: THREE.Vector3; t: number }
  /** Called every frame with the seconds since the last one (particle effects). */
  private readonly updaters = new Set<(dt: number) => void>()

  constructor(private readonly host: HTMLElement) {
    this.attachRenderer()
    // No background colour: the page draws the grey stage behind the canvas, and previews keep their transparency.
    this.scene.add(new THREE.HemisphereLight(0xffffff, 0x404040, 2.2))
    const sun = new THREE.DirectionalLight(0xffffff, 2.2)
    sun.position.set(3, 5, 4)
    this.scene.add(sun)
    this.resize = new ResizeObserver(() => this.fitCanvas())
    this.resize.observe(host)
    this.fitCanvas()
    this.loop()
  }

  /** What to do when the renderer's WebGL context is lost: a new renderer, once the window is visible. */
  private readonly onContextLost = contextRecovery(() => this.replaceRenderer(), {
    hidden: () => document.hidden,
    whenVisible: (fn) => document.addEventListener('visibilitychange', fn, { once: true }),
    later: (fn) => void setTimeout(fn, 0),
    now: () => Date.now()
  })

  /** Makes the renderer and its canvas, and the orbit controls on that canvas (keeping what the previous ones had). */
  private attachRenderer(): void {
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, preserveDrawingBuffer: true })
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))
    renderer.outputColorSpace = THREE.SRGBColorSpace
    this.host.appendChild(renderer.domElement)
    renderer.domElement.addEventListener('webglcontextlost', () => this.onContextLost())
    const previous = this.controls as OrbitControls | undefined
    const controls = new OrbitControls(this.camera, renderer.domElement)
    controls.enableDamping = true
    controls.autoRotateSpeed = 2
    if (previous) {
      controls.target.copy(previous.target)
      controls.autoRotate = previous.autoRotate
      controls.minDistance = previous.minDistance
      controls.maxDistance = previous.maxDistance
      previous.dispose()
    }
    // Dragging takes over from a turn to a view axis.
    controls.addEventListener('start', () => (this.turn = undefined))
    this.renderer = renderer
    this.controls = controls
  }

  /** WebKit reclaims the context of a viewer that sits idle while the window is hidden and many others are made: a fresh renderer draws the same scene again. */
  private replaceRenderer(): void {
    if (this.disposed || !this.renderer.getContext().isContextLost()) return
    const lost = this.renderer
    lost.dispose()
    lost.domElement.remove()
    this.attachRenderer()
    this.fitCanvas()
  }

  /**
   * Parses the FBX; texture names inside it are served from `textures` (keyed
   * by lower-case file name). Resolves once those textures have loaded (or
   * after a short timeout), so a snapshot taken then shows them.
   */
  load(fbx: ArrayBuffer, textures: ReadonlyMap<string, TextureFile>): Promise<void> {
    const manager = new THREE.LoadingManager()
    let started = false
    const loaded = new Promise<void>((resolve) => {
      manager.onLoad = () => resolve()
      setTimeout(resolve, 4000)
    })
    manager.onStart = () => (started = true)
    manager.setURLModifier((url) => textures.get(baseLower(url))?.url ?? BLANK_TEXTURE)
    manager.addHandler(/\.tga$/i, new TGALoader(manager))
    const model = new FBXLoader(manager).parse(fbx, '')
    this.model = model
    this.scene.add(model)
    this.frameModel()
    if (model.animations.length) this.mixer = new THREE.AnimationMixer(model)
    return started ? loaded : Promise.resolve()
  }

  /**
   * Shows a plain shape instead of a model, with one material that
   * `applyMaterialSlots` ('*') and `setLook` dress. Switching shape keeps that material.
   */
  showShape(shape: PreviewShape): void {
    const geometry = shape === 'sphere' ? new THREE.SphereGeometry(0.5, 64, 32) : shape === 'cube' ? new THREE.BoxGeometry(0.7, 0.7, 0.7) : new THREE.PlaneGeometry(1, 1)
    const mesh = this.model?.children[0] as THREE.Mesh | undefined
    if (mesh?.isMesh) {
      mesh.geometry.dispose()
      mesh.geometry = geometry
    } else {
      const group = new THREE.Group()
      group.add(new THREE.Mesh(geometry, new THREE.MeshPhongMaterial({ name: 'preview', side: THREE.DoubleSide })))
      this.model = group
      this.scene.add(group)
    }
    this.model!.position.set(0, 0, 0)
    this.frameModel()
    // A flat card is read face-on.
    if (shape === 'plane') this.setView('front')
  }

  /** Shows a group put together elsewhere (a prefab) like a loaded model; `extent` adds space its meshes do not show (where particles fly). */
  showGroup(group: THREE.Group, extent?: THREE.Box3): void {
    if (this.model) this.scene.remove(this.model)
    this.mixer?.stopAllAction()
    this.mixer = undefined
    this.model = group
    this.scene.add(group)
    this.frameModel(extent)
  }

  /** Runs `fn` every frame until the returned function is called. */
  addUpdater(fn: (dt: number) => void): () => void {
    this.updaters.add(fn)
    return () => this.updaters.delete(fn)
  }

  /** Colour, transparency, cutout, emission and gloss from a Unity material, on top of its textures; `only` limits it to one material by name. */
  setLook(look: MaterialLook, only?: string): void {
    this.model?.traverse((obj) => {
      const mesh = obj as THREE.Mesh
      if (!mesh.isMesh || mesh.userData.particles) return
      for (const mat of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) {
        if (only !== undefined && mat.name !== only) continue
        const m = mat as THREE.MeshPhongMaterial
        // Unity stores colour properties in sRGB; they multiply the colour texture. HDR colours (> 1) would only blow out.
        const c01 = (v: number) => Math.min(1, Math.max(0, v))
        if (look.color) m.color.setRGB(c01(look.color.r), c01(look.color.g), c01(look.color.b), THREE.SRGBColorSpace)
        m.transparent = look.surface === 'transparent'
        m.opacity = m.transparent ? (look.color?.a ?? 1) : 1
        m.depthWrite = !m.transparent
        m.alphaTest = look.surface === 'cutout' ? (look.cutoff ?? 0.5) : 0
        if (look.emission) m.emissive.setRGB(c01(look.emission.r), c01(look.emission.g), c01(look.emission.b), THREE.SRGBColorSpace)
        else m.emissive.set(0x000000)
        const smooth = look.smoothness ?? 0.5
        m.shininess = 4 + smooth * 96
        m.specular.setScalar(0.05 + smooth * 0.35)
        m.needsUpdate = true
      }
    })
  }

  /** Puts .mat textures onto the FBX materials with matching names. */
  async applyMaterialSlots(slots: readonly MaterialSlot[], exact = false): Promise<void> {
    if (!this.model || !slots.length) return
    const byMaterial = new Map<string, MaterialSlot[]>()
    const ordered = [...slots].sort((a, b) => slotRank(a.prop.toLowerCase()) - slotRank(b.prop.toLowerCase()))
    for (const s of ordered) {
      const key = s.material.toLowerCase()
      byMaterial.set(key, [...(byMaterial.get(key) ?? []), s])
    }
    const loadTex = (file: TextureFile, color: boolean) =>
      new Promise<THREE.Texture | undefined>((resolve) => {
        const loader: THREE.Loader<THREE.Texture> = file.name.toLowerCase().endsWith('.tga') ? new TGALoader() : new THREE.TextureLoader()
        loader.load(
          file.url,
          (tex) => {
            if (color) tex.colorSpace = THREE.SRGBColorSpace
            tex.wrapS = tex.wrapT = THREE.RepeatWrapping
            resolve(tex)
          },
          undefined,
          () => resolve(undefined)
        )
      })
    const jobs: Promise<void>[] = []
    this.model.traverse((obj) => {
      const mesh = obj as THREE.Mesh
      if (!mesh.isMesh || mesh.userData.particles) return
      for (const mat of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) {
        const m = mat as THREE.MeshPhongMaterial
        // `exact`: only materials named in the slots (a prefab's materials are named by GUID).
        const slotsFor = exact
          ? byMaterial.get(m.name.toLowerCase())
          : (byMaterial.get(m.name.toLowerCase()) ?? byMaterial.get('*') ?? (byMaterial.size === 1 ? [...byMaterial.values()][0] : undefined))
        let colorTaken = false
        for (const slot of slotsFor ?? []) {
          const prop = slot.prop.toLowerCase()
          const color = COLOR_PROPS.has(prop)
          if (!color && !NORMAL_PROPS.has(prop) && !EMISSION_PROPS.has(prop)) continue
          // One colour map per material: the highest-priority slot.
          if (color && colorTaken) continue
          if (color) colorTaken = true
          jobs.push(
            loadTex(slot.file, color || EMISSION_PROPS.has(prop)).then((tex) => {
              if (!tex || this.disposed) return
              const replace = (old: THREE.Texture | null) => old !== tex && old?.dispose()
              if (color) {
                replace(m.map)
                m.map = tex
                m.color?.set(0xffffff)
              } else if (NORMAL_PROPS.has(prop)) {
                replace(m.normalMap)
                m.normalMap = tex
              } else {
                replace(m.emissiveMap)
                m.emissiveMap = tex
                m.emissive?.set(0xffffff)
              }
              m.needsUpdate = true
            })
          )
        }
      }
    })
    await Promise.all(jobs)
  }

  /** True when at least one material got a colour texture from the FBX itself. */
  hasColorMaps(): boolean {
    let found = false
    this.model?.traverse((obj) => {
      const mesh = obj as THREE.Mesh
      if (!mesh.isMesh || mesh.userData.particles) return
      for (const m of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) if ((m as THREE.MeshPhongMaterial).map) found = true
    })
    return found
  }

  stats(fileBytes: number, textures: readonly TextureFile[]): ModelStats {
    let triangles = 0
    let vertices = 0
    let meshes = 0
    const materials = new Set<string>()
    const bones = new Set<string>()
    this.model?.traverse((obj) => {
      const mesh = obj as THREE.Mesh
      if (!mesh.isMesh || mesh.userData.particles) return
      meshes++
      const geo = mesh.geometry
      const pos = geo.getAttribute('position')
      vertices += pos?.count ?? 0
      triangles += Math.floor((geo.index ? geo.index.count : (pos?.count ?? 0)) / 3)
      for (const m of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) materials.add(m.uuid)
      const skinned = obj as THREE.SkinnedMesh
      if (skinned.isSkinnedMesh) skinned.skeleton.bones.forEach((b) => bones.add(b.uuid))
    })
    return {
      triangles,
      vertices,
      meshes,
      materials: materials.size,
      bones: bones.size,
      animations: this.model?.animations.map((a, i) => a.name || `Clip ${i + 1}`) ?? [],
      textures: textures.map((t) => ({ name: t.name, width: t.width, height: t.height, bytes: t.bytes })),
      fileBytes
    }
  }

  /** The home view, closer (below 1) or farther: the camera keeps its direction to the target. */
  zoomHome(factor: number): void {
    const { target, position } = this.home
    this.camera.position.copy(position).sub(target).multiplyScalar(factor).add(target)
    this.controls.target.copy(target)
    this.controls.update()
  }

  setView(preset: ViewPreset): void {
    const { target, radius } = this.home
    const d = radius * 2.4
    const pos =
      preset === 'front'
        ? new THREE.Vector3(0, radius * 0.4, d)
        : preset === 'side'
          ? new THREE.Vector3(d, radius * 0.4, 0)
          : preset === 'top'
            ? new THREE.Vector3(0, d, 0.0001)
            : this.home.position.clone().sub(target)
    this.turn = undefined
    this.camera.position.copy(target).add(pos)
    this.controls.target.copy(target)
    this.controls.update()
  }

  /** The camera's orientation now (the view gizmo follows it). */
  get cameraQuaternion(): THREE.Quaternion {
    return this.camera.quaternion
  }

  /** Turns the camera around its target, at the same distance, to look from direction `dir` (three.js space). */
  lookFrom(dir: THREE.Vector3): void {
    const to = dir.clone().normalize()
    // Straight up or down, OrbitControls cannot tell which way is up: a hair off.
    if (Math.abs(to.y) > 0.9999) to.set(0, Math.sign(to.y), 0.0001).normalize()
    this.turn = { from: this.camera.position.clone().sub(this.controls.target).normalize(), to, t: 0 }
  }

  /** Which way the camera looks from (from the target to the camera), as a unit vector. */
  get viewDirection(): THREE.Vector3 {
    return this.camera.position.clone().sub(this.controls.target).normalize()
  }

  private stepTurn(dt: number): void {
    const turn = this.turn
    if (!turn) return
    turn.t = Math.min(1, turn.t + dt / 0.25)
    const k = 1 - Math.pow(1 - turn.t, 3)
    const distance = this.camera.position.distanceTo(this.controls.target)
    // Along the great circle between the two directions (opposite ones go round over the top).
    const q = new THREE.Quaternion().setFromUnitVectors(turn.from, turn.to)
    if (turn.from.dot(turn.to) < -0.9999) q.setFromAxisAngle(new THREE.Vector3().crossVectors(turn.from, new THREE.Vector3(0, 1, 0)).normalize(), Math.PI)
    const dir = turn.from.clone().applyQuaternion(new THREE.Quaternion().slerp(q, k))
    this.camera.position.copy(this.controls.target).addScaledVector(dir, distance)
    if (turn.t >= 1) this.turn = undefined
  }

  setAutoRotate(on: boolean): void {
    this.controls.autoRotate = on
  }

  setWireframe(on: boolean): void {
    this.model?.traverse((obj) => {
      const mesh = obj as THREE.Mesh
      if (!mesh.isMesh || mesh.userData.particles) return
      for (const m of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) (m as THREE.MeshPhongMaterial).wireframe = on
    })
  }

  /** Plays a clip made elsewhere (an .anim file) on the shown model, looping or once; null stops. */
  playClip(clip: THREE.AnimationClip | null, loop = true): void {
    this.mixer?.stopAllAction()
    if (!clip || !this.model) return
    this.mixer ??= new THREE.AnimationMixer(this.model)
    const action = this.mixer.clipAction(clip)
    action.setLoop(loop ? THREE.LoopRepeat : THREE.LoopOnce, Infinity)
    action.clampWhenFinished = true
    action.reset().play()
  }

  /** Moves what is playing to `seconds` (for a still pose). */
  poseAt(seconds: number): void {
    this.mixer?.setTime(seconds)
  }

  /** Pauses or resumes whatever is playing. */
  setPaused(paused: boolean): void {
    if (this.mixer) this.mixer.timeScale = paused ? 0 : 1
  }

  /** Plays one animation clip, or stops when `index` is null. */
  playAnimation(index: number | null): void {
    if (!this.mixer || !this.model) return
    this.mixer.stopAllAction()
    if (index !== null && this.model.animations[index]) this.mixer.clipAction(this.model.animations[index]).play()
  }

  /** Small preview image of the current view. */
  snapshot(width = 320, height = 240): string {
    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    this.drawFrame(canvas.getContext('2d')!, 0, 0, width, height)
    return canvas.toDataURL('image/webp', 0.85)
  }

  /** Renders the current view into a cell of another canvas, cropped to fill it (frames of a moving thumbnail), without the grid. */
  drawFrame(ctx: CanvasRenderingContext2D, x: number, y: number, width: number, height: number): void {
    // Captured frames are thumbnails: a flat stage with the soft floor shadow, no grid.
    const grid = this.grid
    const gridWasVisible = grid?.visible
    if (grid) grid.visible = false
    this.renderer.render(this.scene, this.camera)
    if (grid && gridWasVisible !== undefined) grid.visible = gridWasVisible
    const src = this.renderer.domElement
    const scale = Math.max(width / src.width, height / src.height)
    const w = src.width * scale
    const h = src.height * scale
    ctx.save()
    ctx.beginPath()
    ctx.rect(x, y, width, height)
    ctx.clip()
    ctx.drawImage(src, x + (width - w) / 2, y + (height - h) / 2, w, h)
    ctx.restore()
  }

  isDisposed(): boolean {
    return this.disposed
  }

  dispose(): void {
    this.disposed = true
    cancelAnimationFrame(this.frame)
    this.resize.disconnect()
    this.controls.dispose()
    this.scene.traverse((obj) => {
      const mesh = obj as THREE.Mesh
      mesh.geometry?.dispose()
      for (const m of mesh.material ? (Array.isArray(mesh.material) ? mesh.material : [mesh.material]) : []) {
        for (const value of Object.values(m)) if (value instanceof THREE.Texture) value.dispose()
        m.dispose()
      }
    })
    this.renderer.dispose()
    this.renderer.forceContextLoss()
    this.renderer.domElement.remove()
  }

  private frameModel(extent?: THREE.Box3): void {
    if (!this.model) return
    const box = new THREE.Box3().setFromObject(this.model)
    if (extent) box.union(extent)
    if (box.isEmpty()) return
    const size = box.getSize(new THREE.Vector3())
    const center = box.getCenter(new THREE.Vector3())
    // Centre on the origin and stand the model on the grid.
    this.model.position.sub(center).add(new THREE.Vector3(0, size.y / 2, 0))
    const radius = Math.max(size.length() / 2, 1e-3)
    const target = new THREE.Vector3(0, size.y / 2, 0)
    this.camera.near = radius / 100
    this.camera.far = radius * 100
    this.camera.updateProjectionMatrix()
    this.controls.minDistance = radius * 0.2
    this.controls.maxDistance = radius * 20
    this.home = { position: target.clone().add(new THREE.Vector3(radius * 1.6, radius * 0.9, radius * 1.9)), target, radius }
    if (this.grid) this.scene.remove(this.grid)
    this.grid = new THREE.GridHelper(radius * 6, 24, GRID_CENTER, GRID_LINE)
    this.scene.add(this.grid)
    this.setView('reset')
  }

  private fitCanvas(): void {
    const w = Math.max(1, this.host.clientWidth)
    const h = Math.max(1, this.host.clientHeight)
    this.renderer.setSize(w, h, false)
    this.renderer.domElement.style.width = '100%'
    this.renderer.domElement.style.height = '100%'
    this.camera.aspect = w / h
    this.camera.updateProjectionMatrix()
  }

  private loop = (): void => {
    if (this.disposed) return
    this.frame = requestAnimationFrame(this.loop)
    // A frame after the window was hidden can be seconds long: particles would jump.
    const dt = Math.min(this.clock.getDelta(), 0.1)
    this.mixer?.update(dt)
    this.updaters.forEach((fn) => fn(dt))
    this.stepTurn(dt)
    this.controls.update()
    this.renderer.render(this.scene, this.camera)
  }
}
