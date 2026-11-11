import * as THREE from 'three'
import { RoundedBoxGeometry } from 'three/examples/jsm/geometries/RoundedBoxGeometry.js'
import { RoomEnvironment } from 'three/examples/jsm/environments/RoomEnvironment.js'

// The sign-in lobby in 3D: the XP Hero chest on a pedestal over a grid floor,
// loot gems drifting round it. Opening it shakes the chest, swings the lid on
// its hinge, pours light out, and flies the camera down into the light — the
// white-out (CSS, see app.css "lobby") hands over to the hub.

/**
 * How far the lid swings open (radians, about 93°). Its overshoot goes about 7% further (1.73), and the
 * sword behind the chest is first touched at 1.83: wider (it was 1.95) and the lid passed through the blade.
 */
const LID_OPEN = 1.62

/** Seconds into the opening when each part starts; the hub takes over at `end`. */
const T = { shake: 0, lid: 0.6, light: 0.75, beams: 1.0, sparks: 1.1, camera: 1.45, end: 2.9 }

/** Asset type colours (app.css --type-*): one gem each. */
const GEM_COLORS = ['#ffae1f', '#ee5a86', '#4a98f7', '#9479f5', '#25bfa3', '#f5d547', '#6fd38a', '#ff7429']

/**
 * The gems' colours, one set picked at random each time the lobby opens. Only the gems change: the
 * background, the floor and every light keep their original colours ("classic" is the original set).
 */
export interface LobbyTheme {
  name: string
  /** Gems drifting round the chest (eight; the sparks thrown out when it opens take these too), and the ones on the gold (five). */
  gems: readonly string[]
  inside: readonly string[]
}

export const LOBBY_THEMES: readonly LobbyTheme[] = [
  { name: 'classic', gems: GEM_COLORS, inside: ['#e8352b', '#4a98f7', '#25bfa3', '#9479f5', '#ee5a86'] },
  { name: 'dusk', gems: ['#ffae1f', '#ee5a86', '#b07cff', '#ff7aa8', '#f5d547', '#8f6bf0', '#ffc46b', '#e85ab8'], inside: ['#ee5a86', '#b07cff', '#f5d547', '#8f6bf0', '#ff7aa8'] },
  { name: 'lagoon', gems: ['#25bfa3', '#4ad8e8', '#ffae1f', '#4a98f7', '#6fd38a', '#f5d547', '#38b6ff', '#ff8a5c'], inside: ['#25bfa3', '#4a98f7', '#4ad8e8', '#6fd38a', '#ff8a5c'] },
  { name: 'ember', gems: ['#ff7429', '#e8352b', '#ffae1f', '#f5d547', '#ff5a5a', '#ffc46b', '#ee5a86', '#ff9f45'], inside: ['#e8352b', '#ff7429', '#f5d547', '#ee5a86', '#ffc46b'] },
  { name: 'grove', gems: ['#6fd38a', '#25bfa3', '#ffae1f', '#a5e05a', '#f5d547', '#3fbf7f', '#ffc46b', '#4ad8a8'], inside: ['#6fd38a', '#25bfa3', '#f5d547', '#a5e05a', '#e8352b'] }
]

/**
 * Where the camera looks from, as turns round the chest from the original view: degrees to the left (-)
 * or right (+), and up (+) or down (-). Same target and distance, so the chest is framed the same; the
 * sword stands behind the chest's right side, so views stay in front of it, never behind. Each was
 * checked against the original in a screenshot; turning further left (-26°) hid the sword behind the lid.
 */
export const LOBBY_VIEWS: ReadonlyArray<readonly [number, number]> = [
  [0, 0],
  [-14, 2],
  [12, 4],
  [-6, 8]
]

const VIEW_KEY = 'assethub.lobby.view'
const THEME_KEY = 'assethub.lobby.theme'
const LAST_KEY = 'assethub.lobby.last'

/**
 * A different look from last time, at random. `assethub.lobby.theme` in localStorage pins one
 * (by name), for trying a look out.
 */
function pickTheme(): LobbyTheme {
  const read = (key: string) => {
    try {
      return localStorage.getItem(key)
    } catch {
      return null
    }
  }
  const pinned = LOBBY_THEMES.find((t) => t.name === read(THEME_KEY))
  if (pinned) return pinned
  const choices = LOBBY_THEMES.filter((t) => t.name !== read(LAST_KEY))
  const theme = choices[Math.floor(Math.random() * choices.length)] ?? LOBBY_THEMES[0]
  try {
    localStorage.setItem(LAST_KEY, theme.name)
  } catch {
    // A look is still picked; it may repeat next time.
  }
  return theme
}

/** A view at random (`assethub.lobby.view` in localStorage pins one, by index). */
function pickView(): readonly [number, number] {
  try {
    const pinned = localStorage.getItem(VIEW_KEY)
    if (pinned !== null && LOBBY_VIEWS[Number(pinned)]) return LOBBY_VIEWS[Number(pinned)]
  } catch {
    // Random below.
  }
  return LOBBY_VIEWS[Math.floor(Math.random() * LOBBY_VIEWS.length)]
}

/** The original camera position turned round `target` by a view (see `LOBBY_VIEWS`). */
function viewFrom(original: THREE.Vector3, target: THREE.Vector3, [yaw, pitch]: readonly [number, number]): THREE.Vector3 {
  const offset = original.clone().sub(target)
  const spherical = new THREE.Spherical().setFromVector3(offset)
  spherical.theta += THREE.MathUtils.degToRad(yaw)
  spherical.phi = THREE.MathUtils.clamp(spherical.phi - THREE.MathUtils.degToRad(pitch), 0.35, Math.PI / 2 - 0.08)
  return new THREE.Vector3().setFromSpherical(spherical).add(target)
}

/** A small seeded random generator, so one launch's scatter is consistent wherever it is used. */
function seeded(seed: number): () => number {
  let s = Math.max(1, Math.floor(seed)) % 2147483647
  return () => (s = (s * 16807) % 2147483647) / 2147483647
}

const ease = {
  inOut: (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2),
  out: (t: number) => 1 - (1 - t) ** 3,
  outBack: (t: number) => 1 + 2.4 * (t - 1) ** 3 + 1.4 * (t - 1) ** 2,
  in: (t: number) => t * t * t
}
const clamp01 = (t: number) => Math.min(1, Math.max(0, t))
const phase = (t: number, start: number, length: number) => clamp01((t - start) / length)

/** Three bands of light, like the logo's flat cartoon shading. */
function toonRamp(): THREE.DataTexture {
  const tex = new THREE.DataTexture(new Uint8Array([92, 92, 92, 255, 182, 182, 182, 255, 255, 255, 255, 255]), 3, 1, THREE.RGBAFormat)
  tex.minFilter = THREE.NearestFilter
  tex.magFilter = THREE.NearestFilter
  tex.needsUpdate = true
  return tex
}

/** A soft round glow (sparks, pedestal light, the warm haze behind the chest). */
function glowTexture(inner = 'rgba(255,255,255,1)', outer = 'rgba(255,255,255,0)'): THREE.CanvasTexture {
  const c = document.createElement('canvas')
  c.width = c.height = 128
  const g = c.getContext('2d')!
  const grad = g.createRadialGradient(64, 64, 0, 64, 64, 64)
  grad.addColorStop(0, inner)
  grad.addColorStop(1, outer)
  g.fillStyle = grad
  g.fillRect(0, 0, 128, 128)
  const tex = new THREE.CanvasTexture(c)
  tex.colorSpace = THREE.SRGBColorSpace
  return tex
}

/** A cartoon "bling": a four-pointed star with long thin points and a small bright core. */
function blingTexture(): THREE.CanvasTexture {
  const c = document.createElement('canvas')
  c.width = c.height = 128
  const g = c.getContext('2d')!
  const star = (long: number, waist: number) => {
    // Points up, right, down, left; the sides curve in towards the middle, as drawn sparkles do.
    g.beginPath()
    g.moveTo(64, 64 - long)
    g.quadraticCurveTo(64 + waist, 64 - waist, 64 + long, 64)
    g.quadraticCurveTo(64 + waist, 64 + waist, 64, 64 + long)
    g.quadraticCurveTo(64 - waist, 64 + waist, 64 - long, 64)
    g.quadraticCurveTo(64 - waist, 64 - waist, 64, 64 - long)
    g.fill()
  }
  const halo = g.createRadialGradient(64, 64, 0, 64, 64, 30)
  halo.addColorStop(0, 'rgba(255,255,255,0.55)')
  halo.addColorStop(1, 'rgba(255,255,255,0)')
  g.fillStyle = halo
  g.fillRect(0, 0, 128, 128)
  g.fillStyle = '#fff'
  star(62, 7)
  // A smaller star turned 45°, so it reads as a glint from any size.
  g.translate(64, 64)
  g.rotate(Math.PI / 4)
  g.translate(-64, -64)
  g.globalAlpha = 0.7
  star(26, 4)
  const tex = new THREE.CanvasTexture(c)
  tex.colorSpace = THREE.SRGBColorSpace
  return tex
}

/** The light's sides: smooth, bright at the rim and fading as it rises, softer towards each side's edges (no streaks to alias). */
function beamTexture(): THREE.CanvasTexture {
  const c = document.createElement('canvas')
  c.width = 128
  c.height = 256
  const g = c.getContext('2d')!
  const up = g.createLinearGradient(0, 256, 0, 0)
  up.addColorStop(0, 'rgba(255,240,190,1)')
  up.addColorStop(0.35, 'rgba(255,220,140,0.55)')
  up.addColorStop(1, 'rgba(255,200,90,0)')
  g.fillStyle = up
  g.fillRect(0, 0, 128, 256)
  // Edges dimmed with a horizontal mask, so the box reads as a glowing column, not flat panels.
  g.globalCompositeOperation = 'destination-in'
  const across = g.createLinearGradient(0, 0, 128, 0)
  across.addColorStop(0, 'rgba(0,0,0,0.35)')
  across.addColorStop(0.5, 'rgba(0,0,0,1)')
  across.addColorStop(1, 'rgba(0,0,0,0.35)')
  g.fillStyle = across
  g.fillRect(0, 0, 128, 256)
  const tex = new THREE.CanvasTexture(c)
  tex.colorSpace = THREE.SRGBColorSpace
  return tex
}

export class LobbyScene {
  private readonly renderer: THREE.WebGLRenderer
  private readonly scene = new THREE.Scene()
  private readonly camera = new THREE.PerspectiveCamera(36, 1, 0.05, 100)
  private readonly chest = new THREE.Group()
  private readonly lid = new THREE.Group()
  private readonly inner: THREE.Mesh<THREE.PlaneGeometry, THREE.MeshBasicMaterial>
  /** Short reach: it lights the open lid and the air above, without bleaching the chest's own colours. */
  private readonly chestLight = new THREE.PointLight('#ffd27a', 0, 3.2, 2)
  /** The light: a box as wide and deep as the chest's mouth, bright at the rim and fading as it rises. */
  private readonly beams: THREE.Mesh<THREE.BoxGeometry, THREE.MeshBasicMaterial[]>
  private readonly beamMat: THREE.MeshBasicMaterial
  /** Soft studio reflections, for the sword's blade only (the rest stays flat cartoon). */
  private readonly env: THREE.Texture
  private readonly pedestalGlow: THREE.Mesh<THREE.PlaneGeometry, THREE.MeshBasicMaterial>
  private readonly gems: Array<{ mesh: THREE.Mesh; base: THREE.Vector3; phase: number }> = []
  /** The band's two ends hanging from its knot: they sway a little with the chest (more while it shakes). */
  private readonly knotEnds: Array<{ pivot: THREE.Object3D; base: number; phase: number }> = []
  private readonly sparks: THREE.Points<THREE.BufferGeometry, THREE.PointsMaterial>
  private readonly sparkVel: Float32Array
  /** Blings (four-pointed sparkles) on the gold inside, showing through the lifted lid while START is hovered. */
  private readonly glints: THREE.Points<THREE.BufferGeometry, THREE.PointsMaterial>
  private readonly glintPhase: Float32Array
  /** 0…1, eased towards `hoverTarget`: how much the chest is reacting to the hovered START button. */
  private hover = 0
  private hoverTarget = 0
  private readonly resize: ResizeObserver
  private readonly disposables: Array<{ dispose(): void }> = []
  private frame = 0
  private readonly clock = new THREE.Clock()
  private openedAt?: number
  private readonly still: boolean
  /** Idle framing: the chest sits in the upper part, over the title and the button. */
  /** A three-quarter view from a little above: the lid's dome and a side show, not just the front. */
  /** The chest's middle, sword and pedestal included, and how tall that is. */
  private readonly idleTarget = new THREE.Vector3(0, 1.45, 0)
  /** The original view (3.4, 4.6, 10.6), turned by this opening's pick of `LOBBY_VIEWS`. */
  private readonly idlePos = viewFrom(new THREE.Vector3(3.4, 4.6, 10.6), this.idleTarget, pickView())
  private readonly chestHeight = 3.9
  /** Where on the canvas the chest goes (the page's room for it): centre and height in px. */
  private slot?: { centerY: number; height: number }
  /** One gentle arc from wherever the camera is when the chest opens down to just over the light (see `open`). */
  private path?: THREE.QuadraticBezierCurve3
  private readonly pathEnd = new THREE.Vector3(0, 2.2, 0.15)
  /** This opening's gem colours and the seed of its scatter (see `LobbyTheme`). */
  private readonly theme = pickTheme()
  private readonly scatter = seeded(1 + Math.random() * 2147483000)

  constructor(private readonly canvas: HTMLCanvasElement) {
    this.still = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true })
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))
    this.renderer.outputColorSpace = THREE.SRGBColorSpace
    // No tone mapping: the logo's flat colours come out as they are, not washed towards grey.
    this.renderer.toneMapping = THREE.NoToneMapping

    const bg = new THREE.Color('#0b1017')
    this.scene.background = bg
    this.scene.fog = new THREE.Fog(bg, 9, 24)

    // Light: a warm key from the front left does the toon banding; a faint cool fill keeps shadows readable,
    // an orange rim from behind lifts the silhouette off the dark.
    this.scene.add(new THREE.HemisphereLight('#9fb4d6', '#0b1017', 0.35))
    const key = new THREE.DirectionalLight('#fff4de', 2.6)
    key.position.set(-4, 6, 7)
    this.scene.add(key)
    const fill = new THREE.DirectionalLight('#8fb0ff', 0.45)
    fill.position.set(5, 2, 4)
    this.scene.add(fill)
    // With no ink lines, the rim light is what sets the chest off the dark: a touch stronger.
    const rim = new THREE.DirectionalLight('#ffae1f', 2.6)
    rim.position.set(1.5, 3, -6)
    this.scene.add(rim)

    const pmrem = new THREE.PMREMGenerator(this.renderer)
    this.env = pmrem.fromScene(new RoomEnvironment(), 0.04).texture
    pmrem.dispose()
    this.disposables.push(this.env)

    this.buildFloor()
    this.buildChest()
    this.scene.add(this.chest)

    // Inside the chest: a glowing surface under the lid, and a light that wakes up when it opens.
    // A soft glow rising off the gold once the lid is up.
    this.inner = new THREE.Mesh(new THREE.PlaneGeometry(2.2, 1.4), new THREE.MeshBasicMaterial({ map: glowTexture('rgba(255,236,170,1)', 'rgba(255,190,60,0)'), transparent: true, opacity: 0, blending: THREE.AdditiveBlending, depthWrite: false }))
    this.inner.rotation.x = -Math.PI / 2
    this.inner.position.y = 1.12
    this.chest.add(this.inner)
    this.chestLight.position.set(0, 1.45, -0.1)
    this.chest.add(this.chestLight)

    // Light out of the chest: the four sides of a box filling the mouth (no top or bottom), fading upwards;
    // seen from above, its front and back overlap into a solid-looking column of light.
    const beamTex = beamTexture()
    this.beamMat = new THREE.MeshBasicMaterial({ map: beamTex, transparent: true, opacity: 0, blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide, fog: false })
    const none = new THREE.MeshBasicMaterial({ visible: false })
    const beamGeo = new THREE.BoxGeometry(2.2, 4.2, 1.34)
    beamGeo.translate(0, 2.1, 0)
    // BoxGeometry's faces: +x, -x, +y, -y, +z, -z.
    this.beams = new THREE.Mesh(beamGeo, [this.beamMat, this.beamMat, none, none, this.beamMat, this.beamMat])
    this.beams.position.set(0, 1.15, 0)
    this.beams.scale.set(1, 0.001, 1)
    this.chest.add(this.beams)

    // Warm haze behind the chest.
    const haze = new THREE.Sprite(new THREE.SpriteMaterial({ map: glowTexture('rgba(255,174,31,.38)', 'rgba(255,174,31,0)'), depthWrite: false, fog: false }))
    haze.scale.set(9, 7, 1)
    haze.position.set(0, 1.6, -2.5)
    this.scene.add(haze)

    // Pedestal glow, pulsing gently.
    this.pedestalGlow = new THREE.Mesh(new THREE.PlaneGeometry(5.5, 5.5), new THREE.MeshBasicMaterial({ map: glowTexture('rgba(255,174,31,.75)', 'rgba(255,174,31,0)'), transparent: true, blending: THREE.AdditiveBlending, depthWrite: false }))
    this.pedestalGlow.rotation.x = -Math.PI / 2
    this.pedestalGlow.position.y = 0.02
    this.scene.add(this.pedestalGlow)

    this.buildGems()

    // Sparks thrown up out of the chest once it is open.
    const count = 70
    const positions = new Float32Array(count * 3)
    const colors = new Float32Array(count * 3)
    this.sparkVel = new Float32Array(count * 4)
    const tint = new THREE.Color()
    for (let i = 0; i < count; i++) {
      positions.set([0, -50, 0], i * 3)
      tint.set(i % 3 === 0 ? '#fff3c4' : this.theme.gems[i % this.theme.gems.length])
      colors.set([tint.r, tint.g, tint.b], i * 3)
      const a = Math.random() * Math.PI * 2
      const spread = 0.6 + Math.random() * 1.6
      // x, y, z velocity and a start delay.
      this.sparkVel.set([Math.cos(a) * spread, 3 + Math.random() * 3.5, Math.sin(a) * spread * 0.7, Math.random() * 0.8], i * 4)
    }
    const sparkGeo = new THREE.BufferGeometry()
    sparkGeo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    sparkGeo.setAttribute('color', new THREE.BufferAttribute(colors, 3))
    this.sparks = new THREE.Points(sparkGeo, new THREE.PointsMaterial({ size: 0.16, map: glowTexture(), vertexColors: true, transparent: true, blending: THREE.AdditiveBlending, depthWrite: false, opacity: 0 }))
    this.chest.add(this.sparks)

    const GLINTS = 18
    const glintPos = new Float32Array(GLINTS * 3)
    this.glintPhase = new Float32Array(GLINTS)
    for (let i = 0; i < GLINTS; i++) {
      // On the gold just under the rim (chest 2.5 wide, 1.25 high, 1.65 deep): seen through the gap under the lid.
      glintPos.set([(Math.random() * 2 - 1) * 0.95, 1.2 + Math.random() * 0.12, (Math.random() * 2 - 1) * 0.5], i * 3)
      this.glintPhase[i] = Math.random() * Math.PI * 2
    }
    const glintGeo = new THREE.BufferGeometry()
    glintGeo.setAttribute('position', new THREE.BufferAttribute(glintPos, 3))
    glintGeo.setAttribute('color', new THREE.BufferAttribute(new Float32Array(GLINTS * 3), 3))
    this.glints = new THREE.Points(glintGeo, new THREE.PointsMaterial({ size: 0.56, map: blingTexture(), vertexColors: true, transparent: true, blending: THREE.AdditiveBlending, depthWrite: false }))
    this.chest.add(this.glints)

    this.disposables.push(beamTex)
    this.resize = new ResizeObserver(() => this.fit())
    this.resize.observe(canvas)
    this.fit()
    this.loop()
  }

  /**
   * Opens the chest and flies the camera into it. Resolves when the light has
   * filled the screen, which is when the hub should take over.
   */
  open(): Promise<void> {
    this.openedAt = this.clock.getElapsedTime()
    // Starts where the idle sway has left the camera, so the move picks up without a jump; the control point
    // lifts the arc a little over the straight line, in front of the chest (the sword stands behind it).
    const from = this.camera.position.clone()
    const control = from.clone().lerp(this.pathEnd, 0.5).add(new THREE.Vector3(0, 0.9, 0))
    this.path = new THREE.QuadraticBezierCurve3(from, control, this.pathEnd.clone())
    const end = this.still ? 0.4 : T.end
    return new Promise((resolve) => window.setTimeout(resolve, end * 1000))
  }

  /** START is hovered or focused: the chest wobbles, peeks open and glints, as if eager to be opened. */
  setHover(on: boolean): void {
    this.hoverTarget = on ? 1 : 0
  }

  /** Frames the chest into this band of the canvas, over the title and the button. */
  setSlot(centerY: number, height: number): void {
    this.slot = { centerY, height }
  }

  dispose(): void {
    cancelAnimationFrame(this.frame)
    this.resize.disconnect()
    this.scene.traverse((o) => {
      const mesh = o as THREE.Mesh
      mesh.geometry?.dispose()
      const mats = Array.isArray(mesh.material) ? mesh.material : mesh.material ? [mesh.material] : []
      for (const m of mats) {
        ;(m as THREE.MeshBasicMaterial).map?.dispose()
        m.dispose()
      }
    })
    for (const d of this.disposables) d.dispose()
    this.renderer.dispose()
  }

  private fit(): void {
    const w = this.canvas.clientWidth || 1
    const h = this.canvas.clientHeight || 1
    this.renderer.setSize(w, h, false)
    this.camera.aspect = w / h
    this.camera.updateProjectionMatrix()
  }

  /** A part with a black outline round it (the inverted hull trick), like the logo's ink. */
  /** One piece of the chest in flat cartoon shading: no outlines — in 3D they only showed up as stray lines at joints. */
  private part(geometry: THREE.BufferGeometry, color: string, parent: THREE.Object3D, ramp: THREE.Texture): THREE.Mesh {
    const mesh = new THREE.Mesh(geometry, new THREE.MeshToonMaterial({ color, gradientMap: ramp }))
    parent.add(mesh)
    return mesh
  }

  private buildChest(): void {
    const ramp = toonRamp()
    this.disposables.push(ramp)
    const W = 2.5
    const H = 1.25
    const D = 1.65
    const round = (w: number, h: number, d: number, r: number) => new RoundedBoxGeometry(w, h, d, 4, r)
    // A hollow body: four walls and a floor, lined dark inside so it has depth; its darker foot.
    const T = 0.14
    for (const z of [D / 2 - T / 2, -(D / 2 - T / 2)]) {
      const wall = this.part(round(W, H, T, 0.06), '#ffc21a', this.chest, ramp)
      wall.position.set(0, H / 2, z)
    }
    for (const x of [W / 2 - T / 2, -(W / 2 - T / 2)]) {
      const wall = this.part(round(T, H, D - 0.02, 0.06), '#ffc21a', this.chest, ramp)
      wall.position.set(x, H / 2, 0)
    }
    const lining = new THREE.MeshToonMaterial({ color: '#4a2610', gradientMap: ramp, side: THREE.DoubleSide })
    const innerW = W - 2 * T - 0.01
    const innerD = D - 2 * T - 0.01
    for (const [w, x, z, ry] of [[innerW, 0, innerD / 2, 0], [innerW, 0, -innerD / 2, 0], [innerD, innerW / 2, 0, Math.PI / 2], [innerD, -innerW / 2, 0, Math.PI / 2]] as const) {
      const panel = new THREE.Mesh(new THREE.PlaneGeometry(w, H - 0.36), lining)
      panel.position.set(x, 0.36 + (H - 0.36) / 2, z)
      panel.rotation.y = ry
      this.chest.add(panel)
    }
    const floor = new THREE.Mesh(new THREE.BoxGeometry(innerW, 0.06, innerD), lining)
    floor.position.y = 0.36
    this.chest.add(floor)
    const foot = this.part(round(W + 0.04, 0.34, D + 0.04, 0.09), '#e8980c', this.chest, ramp)
    foot.position.y = 0.17
    this.buildGold(ramp, innerW, innerD, H)

    // Leather straps over the front and back, gold rivets on the front.
    const rivetGeo = new THREE.SphereGeometry(0.055, 16, 12)
    for (const x of [-0.82, 0.82]) {
      for (const z of [D / 2 + 0.025, -D / 2 - 0.025]) {
        const strap = this.part(round(0.34, H + 0.03, 0.07, 0.03), '#7a3e2e', this.chest, ramp)
        strap.position.set(x, H / 2, z)
      }
      for (const y of [0.32, 0.86]) {
        const rivet = this.part(rivetGeo, '#ffd98a', this.chest, ramp)
        rivet.position.set(x, y, D / 2 + 0.07)
      }
    }
    // A red cloth band round the rim, knotted on the right.
    this.buildBand(ramp, W, H, D)
    // The lock: a rounded gold plate with a keyhole.
    const lock = this.part(round(0.48, 0.58, 0.16, 0.07), '#f5a12e', this.chest, ramp)
    lock.position.set(0, H - 0.12, D / 2 + 0.1)
    const ink = new THREE.MeshBasicMaterial({ color: '#141414' })
    const hole = new THREE.Mesh(new THREE.CircleGeometry(0.06, 20), ink)
    hole.position.set(0, 0.04, 0.081)
    const slit = new THREE.Mesh(new THREE.PlaneGeometry(0.05, 0.14), ink)
    slit.position.set(0, -0.06, 0.081)
    lock.add(hole, slit)

    // The lid: a half cylinder on a hinge along the back edge, straps over it, a highlight on its crown.
    this.lid.position.set(0, H + 0.1, -D / 2)
    this.chest.add(this.lid)
    const lidGeo = new THREE.CylinderGeometry(D / 2, D / 2, W, 48, 1, false, 0, Math.PI)
    lidGeo.rotateZ(Math.PI / 2)
    const top = this.part(lidGeo, '#ffd23f', this.lid, ramp)
    top.position.z = D / 2
    // Open, its hollow shows: painted inside too, not the black outline behind it.
    ;(top.material as THREE.MeshToonMaterial).side = THREE.DoubleSide
    const capGeo = new THREE.CircleGeometry(D / 2, 48, 0, Math.PI)
    for (const side of [-1, 1]) {
      const cap = this.part(capGeo, '#ffd23f', this.lid, ramp)
      cap.rotation.y = (side * Math.PI) / 2
      cap.position.set((side * W) / 2, 0, D / 2)
      ;(cap.material as THREE.MeshToonMaterial).side = THREE.DoubleSide
    }
    for (const x of [-0.82, 0.82]) {
      const strapGeo = new THREE.CylinderGeometry(D / 2 + 0.035, D / 2 + 0.035, 0.34, 48, 1, true, 0, Math.PI)
      strapGeo.rotateZ(Math.PI / 2)
      const strap = this.part(strapGeo, '#7a3e2e', this.lid, ramp)
      strap.position.set(x, 0, D / 2)
      ;(strap.material as THREE.MeshToonMaterial).side = THREE.DoubleSide
    }
    const shineGeo = new THREE.CylinderGeometry(D / 2 + 0.01, D / 2 + 0.01, 0.62, 24, 1, true, 0.55, 0.5)
    shineGeo.rotateZ(Math.PI / 2)
    const shine = new THREE.Mesh(shineGeo, new THREE.MeshBasicMaterial({ color: '#ffe680', side: THREE.DoubleSide }))
    shine.position.set(-0.32, 0, D / 2)
    this.lid.add(shine)

    // The ice sword standing behind the chest's right side, tilted right as in the logo, its blade above the lid —
    // far enough back that the lid swings open clear of it (see LID_OPEN).
    const sword = new THREE.Group()
    // The blade in one piece, straight edges to a point, bevelled so its facets catch the light.
    const outlineShape = new THREE.Shape()
    outlineShape.moveTo(-0.13, 0.12)
    outlineShape.lineTo(0.13, 0.12)
    outlineShape.lineTo(0.13, 1.55)
    outlineShape.lineTo(0, 1.95)
    outlineShape.lineTo(-0.13, 1.55)
    outlineShape.closePath()
    const bladeGeo = new THREE.ExtrudeGeometry(outlineShape, { depth: 0.02, bevelEnabled: true, bevelThickness: 0.035, bevelSize: 0.04, bevelSegments: 1 })
    bladeGeo.translate(0, 0, -0.01)
    bladeGeo.computeVertexNormals()
    // Icy steel: a little metal and gloss, with soft reflections of a studio round it.
    const steel = new THREE.MeshStandardMaterial({ color: '#c4e2ff', metalness: 0.7, roughness: 0.14, envMap: this.env, envMapIntensity: 1.5, flatShading: true })
    const blade = new THREE.Mesh(bladeGeo, steel)
    sword.add(blade)
    // A bright fuller down the middle of each face.
    for (const side of [1, -1]) {
      const fuller = new THREE.Mesh(new THREE.PlaneGeometry(0.035, 1.25), new THREE.MeshStandardMaterial({ color: '#ffffff', metalness: 0.2, roughness: 0.1, envMap: this.env }))
      fuller.position.set(0, 0.82, side * 0.047)
      if (side < 0) fuller.rotation.y = Math.PI
      sword.add(fuller)
    }
    const guard = this.part(round(0.7, 0.17, 0.2, 0.06), '#f5a12e', sword, ramp)
    guard.position.y = 0.08
    const grip = this.part(new THREE.CylinderGeometry(0.07, 0.07, 0.36, 16), '#7a3e2e', sword, ramp)
    grip.position.y = -0.18
    const pommel = this.part(new THREE.SphereGeometry(0.1, 16, 12), '#f5a12e', sword, ramp)
    pommel.position.y = -0.4
    sword.scale.setScalar(1.3)
    sword.position.set(0.85, 0.55, -D / 2 - 0.98)
    sword.rotation.set(-0.12, 0, -0.36)
    this.chest.add(sword)
  }

  /**
   * The cloth band round the chest's rim: one flat strip hugging the walls with rounded corners (it
   * also covers the seam under the lid), knotted on the right side: a bunched knot of the same cloth
   * with a piece of the band wrapped across it, and two soft ends hanging from it (on pivots, so the
   * loop can sway them: `knotEnds`).
   */
  private buildBand(ramp: THREE.Texture, W: number, H: number, D: number): void {
    const cloth = new THREE.MeshToonMaterial({ color: '#e8352b', gradientMap: ramp, side: THREE.DoubleSide })
    const shade = new THREE.MeshToonMaterial({ color: '#c92a22', gradientMap: ramp, side: THREE.DoubleSide })
    const BAND = 0.24
    const OUT = 0.05
    /** A rounded rectangle centred on the chest, `half` its half sizes, as a 2D path (x, z). */
    const rounded = (hw: number, hd: number, rc: number) => {
      const p = new THREE.Shape()
      p.moveTo(-hw + rc, -hd)
      p.lineTo(hw - rc, -hd)
      p.quadraticCurveTo(hw, -hd, hw, -hd + rc)
      p.lineTo(hw, hd - rc)
      p.quadraticCurveTo(hw, hd, hw - rc, hd)
      p.lineTo(-hw + rc, hd)
      p.quadraticCurveTo(-hw, hd, -hw, hd - rc)
      p.lineTo(-hw, -hd + rc)
      p.quadraticCurveTo(-hw, -hd, -hw + rc, -hd)
      return p
    }
    // The band: the ring between the walls and just outside them, extruded up to the band's height.
    const ring = rounded(W / 2 + OUT, D / 2 + OUT, 0.12)
    ring.holes.push(rounded(W / 2 - 0.02, D / 2 - 0.02, 0.06) as unknown as THREE.Path)
    const bandGeo = new THREE.ExtrudeGeometry(ring, { depth: BAND, bevelEnabled: true, bevelThickness: 0.015, bevelSize: 0.015, bevelSegments: 2, curveSegments: 10 })
    bandGeo.rotateX(Math.PI / 2)
    bandGeo.translate(0, H + BAND / 2, 0)
    this.chest.add(new THREE.Mesh(bandGeo, cloth))

    // The knot, on the band on the right side, facing out.
    const knot = new THREE.Group()
    knot.position.set(W / 2 + OUT + 0.06, H, 0.4)
    knot.rotation.y = Math.PI / 2
    this.chest.add(knot)
    // The two ends behind the knot: soft strips that hang, curl out a little and end on a slant.
    for (const [side, length] of [[-1, 0.5], [1, 0.4]] as const) {
      const geo = new THREE.PlaneGeometry(0.17, length, 1, 14)
      geo.translate(0, -length / 2, 0)
      const pos = geo.attributes.position as THREE.BufferAttribute
      for (let k = 0; k < pos.count; k++) {
        const y = pos.getY(k)
        const t = -y / length
        // Bellying out from the chest as it falls, a slight twist, and the tip cut at a slant.
        pos.setZ(k, 0.1 * Math.sin(t * Math.PI * 0.9) + pos.getX(k) * 0.25 * t)
        if (t > 0.98) pos.setY(k, y + side * pos.getX(k) * 0.35)
      }
      geo.computeVertexNormals()
      const pivot = new THREE.Group()
      pivot.position.set(side * 0.05, -0.06, -0.02)
      pivot.rotation.z = side * 0.2
      knot.add(pivot)
      pivot.add(new THREE.Mesh(geo, side < 0 ? cloth : shade))
      this.knotEnds.push({ pivot, base: side * 0.2, phase: side < 0 ? 0 : 1.3 })
    }
    // The knot itself: cloth bunched into a soft lump, the band wrapped tight across its middle.
    const lump = new THREE.Mesh(new THREE.SphereGeometry(0.15, 20, 14), cloth)
    lump.scale.set(1.25, 0.95, 0.7)
    knot.add(lump)
    const wrap = new THREE.Mesh(new RoundedBoxGeometry(0.11, 0.3, 0.24, 3, 0.04), shade)
    wrap.rotation.z = 0.35
    wrap.position.z = 0.01
    knot.add(wrap)
  }

  /** The treasure inside: a mound of gold, coins strewn over it at odd tilts, a few gems — below the rim, so the chest has depth. */
  private buildGold(ramp: THREE.Texture, innerW: number, innerD: number, H: number): void {
    const a = innerW / 2 - 0.02
    const b = innerD / 2 - 0.02
    // Heaped up: the middle rises a little over the rim, the edges stay below it so the walls still show depth.
    const base = H - 0.46
    const rise = 0.58
    const mound = new THREE.Mesh(new THREE.SphereGeometry(1, 40, 20, 0, Math.PI * 2, 0, Math.PI / 2), new THREE.MeshToonMaterial({ color: '#ffc928', gradientMap: ramp }))
    mound.scale.set(a, rise, b)
    mound.position.y = base
    this.chest.add(mound)
    const fill = new THREE.Mesh(new THREE.BoxGeometry(innerW, base - 0.36, innerD), mound.material)
    fill.position.y = 0.36 + (base - 0.36) / 2
    this.chest.add(fill)
    // Height of the mound's surface over a point inside it.
    const surface = (x: number, z: number) => base + rise * Math.sqrt(Math.max(0, 1 - (x / a) ** 2 - (z / b) ** 2))
    const COINS = 150
    const SPILLED = 9
    const coinGeo = new THREE.CylinderGeometry(0.1, 0.1, 0.03, 20)
    const coinMat = new THREE.MeshToonMaterial({ color: '#ffd84a', gradientMap: ramp })
    const coins = new THREE.InstancedMesh(coinGeo, coinMat, COINS)
    const m = new THREE.Matrix4()
    const q = new THREE.Quaternion()
    const e = new THREE.Euler()
    // Strewn differently each time.
    const rand = this.scatter
    for (let i = 0; i < COINS; i++) {
      // Denser towards the middle, where the heap is.
      const r = Math.sqrt(rand()) * 0.95
      const ang = rand() * Math.PI * 2
      const x = Math.cos(ang) * r * a
      const z = Math.sin(ang) * r * b
      q.setFromEuler(e.set((rand() - 0.5) * 1.1, rand() * Math.PI, (rand() - 0.5) * 1.1))
      m.compose(new THREE.Vector3(x, surface(x, z) + 0.01, z), q, new THREE.Vector3(1, 1, 1))
      coins.setMatrixAt(i, m)
      // Some coins catch the light more than others.
      coins.setColorAt(i, new THREE.Color(rand() < 0.3 ? '#fff0a0' : '#ffd84a'))
    }
    this.chest.add(coins)
    // A few spilled onto the pedestal in front: part of the scene, not the chest, so they lie still on the
    // stone while the chest bobs and shakes. Lying flat or barely tilted, resting on it (or on the coin below).
    const spilled = new THREE.InstancedMesh(coinGeo, coinMat, SPILLED)
    let below = new THREE.Vector3()
    for (let i = 0; i < SPILLED; i++) {
      const stack = i % 3 === 2 ? 1 : 0
      const x = stack ? below.x + 0.03 : (rand() * 2 - 1) * 1.2
      const z = stack ? below.z - 0.02 : innerD / 2 + 0.45 + rand() * 0.45
      const tilt = stack ? 0.12 : 0.04
      q.setFromEuler(e.set((rand() - 0.5) * tilt, rand() * Math.PI, (rand() - 0.5) * tilt))
      // Half its thickness, plus what a tilt lifts its rim by, plus the coin underneath when stacked.
      const y = 0.015 + 0.1 * Math.sin(tilt / 2) + stack * 0.03
      const at = new THREE.Vector3(x, y, z)
      below = at
      m.compose(at, q, new THREE.Vector3(1, 1, 1))
      spilled.setMatrixAt(i, m)
    }
    this.scene.add(spilled)
    const gemGeo = new THREE.OctahedronGeometry(0.12, 0)
    const spots = [[-0.45, 0.1], [0.35, -0.15], [0.05, 0.25], [-0.15, -0.3], [0.62, 0.2]] as const
    for (const [i, [x, z]] of spots.entries()) {
      const color = this.theme.inside[i % this.theme.inside.length]
      const gem = new THREE.Mesh(gemGeo, new THREE.MeshStandardMaterial({ color, emissive: color, emissiveIntensity: 0.5, roughness: 0.2, flatShading: true }))
      gem.position.set(x, surface(x, z) + 0.06, z)
      gem.rotation.set(0.4, x * 3, 0.2)
      this.chest.add(gem)
    }
  }

  private buildFloor(): void {
    const grid = new THREE.GridHelper(48, 48, '#3a4659', '#253041')
    ;(grid.material as THREE.Material).transparent = true
    ;(grid.material as THREE.Material).opacity = 0.75
    this.scene.add(grid)
    // A stone pedestal with a gold rim and an inset top.
    const pedestal = new THREE.Mesh(new THREE.CylinderGeometry(1.8, 2.0, 0.26, 72), new THREE.MeshStandardMaterial({ color: '#273142', roughness: 0.85 }))
    pedestal.position.y = -0.13
    this.scene.add(pedestal)
    const inset = new THREE.Mesh(new THREE.CircleGeometry(1.55, 72), new THREE.MeshStandardMaterial({ color: '#1c2431', roughness: 0.9 }))
    inset.rotation.x = -Math.PI / 2
    inset.position.y = 0.005
    this.scene.add(inset)
    const rim = new THREE.Mesh(new THREE.TorusGeometry(1.8, 0.045, 10, 120), new THREE.MeshStandardMaterial({ color: '#ffae1f', emissive: '#b8720a', emissiveIntensity: 0.8, roughness: 0.4, metalness: 0.6 }))
    rim.rotation.x = Math.PI / 2
    rim.position.y = 0.0
    this.scene.add(rim)
  }

  private buildGems(): void {
    const geo = new THREE.OctahedronGeometry(1, 0)
    // The colours in a different order each time, and each gem a little nearer or further, higher or lower.
    const colors = [...this.theme.gems].sort(() => this.scatter() - 0.5)
    colors.forEach((color, i) => {
      // Offset so none drifts across the sword (the angles stay; only radius and height vary).
      const a = (i / colors.length) * Math.PI * 2 + 0.95
      const radius = 3.2 + (i % 3) * 0.9 + (this.scatter() - 0.5) * 0.4
      // Ones that come round to the front keep to the sides, never across the chest.
      const x = Math.cos(a) * radius
      const z = Math.sin(a) * radius * 0.6 - 1.6
      const base = new THREE.Vector3(z > -0.4 ? Math.sign(x || 1) * Math.max(Math.abs(x), 2.7) : x, 1 + Math.floor(this.scatter() * 5) * 0.55, z)
      // Cut facets in full colour, lit from within a little so they read as gems, not plastic.
      const mesh = new THREE.Mesh(geo, new THREE.MeshStandardMaterial({ color, emissive: color, emissiveIntensity: 0.45, roughness: 0.18, metalness: 0.05, flatShading: true }))
      mesh.scale.set(0.15 + (i % 4) * 0.04, (0.15 + (i % 4) * 0.04) * 1.35, 0.15 + (i % 4) * 0.04)
      mesh.position.copy(base)
      this.scene.add(mesh)
      this.gems.push({ mesh, base, phase: i * 1.7 + this.scatter() * 0.6 })
    })
  }

  private loop = (): void => {
    this.frame = requestAnimationFrame(this.loop)
    if (document.hidden) return
    const now = this.clock.getElapsedTime()
    const idle = this.still ? 0 : now
    const t = this.openedAt === undefined ? -1 : now - this.openedAt

    for (const g of this.gems) {
      g.mesh.position.y = g.base.y + Math.sin(idle * 0.9 + g.phase) * 0.18
      g.mesh.rotation.y = idle * 0.8 + g.phase
      g.mesh.rotation.x = 0.4
    }
    this.pedestalGlow.material.opacity = 0.55 + Math.sin(idle * 2) * 0.2
    this.chest.position.y = Math.sin(idle * 1.6) * 0.05
    // The band's ends swing gently, out of step, harder while the chest rattles (hover) or opens.
    const swing = 0.07 + this.hover * 0.12 + (t >= 0 && t < T.lid + 0.4 ? 0.18 : 0)
    for (const end of this.knotEnds) end.pivot.rotation.z = end.base + Math.sin(idle * 2.1 + end.phase) * swing

    // Hovering START: a little rattle and hop, the lid lifting off its rim with light and blings inside.
    this.hover += ((t < 0 && !this.still ? this.hoverTarget : 0) - this.hover) * 0.12
    const hv = this.hover
    if (t < 0) {
      this.chest.rotation.z = Math.sin(now * 10) * 0.025 * hv
      this.chest.position.y += Math.abs(Math.sin(now * 5)) * 0.07 * hv
      this.lid.rotation.x = -0.24 * hv * (0.65 + 0.35 * Math.sin(now * 5))
      this.inner.material.opacity = 0.4 * hv
      this.chestLight.intensity = 1.8 * hv
    }
    const glow = this.glints.geometry.getAttribute('color') as THREE.BufferAttribute
    for (let i = 0; i < this.glintPhase.length; i++) {
      // Short, separate twinkles rather than a steady shine.
      const b = hv * Math.max(0, Math.sin(now * 3.2 + this.glintPhase[i])) ** 8
      glow.setXYZ(i, b, b * 0.92, b * 0.7)
    }
    glow.needsUpdate = true

    // Idle camera: a slow sway; the view shifted so the chest sits above the title.
    const w = this.canvas.clientWidth || 1
    const h = this.canvas.clientHeight || 1
    // Fit the chest's height into its slot, its middle on the slot's middle.
    const slot = this.slot ?? { centerY: h * 0.32, height: h * 0.4 }
    const distance = this.idlePos.distanceTo(this.idleTarget)
    const fitFov = THREE.MathUtils.radToDeg(2 * Math.atan((this.chestHeight * h) / (2 * distance * Math.max(60, slot.height))))
    const baseFov = THREE.MathUtils.clamp(fitFov, 18, 70)
    const baseShift = (h / 2 - slot.centerY) / h
    let shift = baseShift
    this.camera.position.copy(this.idlePos).add(new THREE.Vector3(Math.sin(idle * 0.25) * 0.5, Math.sin(idle * 0.2) * 0.12, 0))
    const target = this.idleTarget.clone()
    this.camera.fov = baseFov

    if (t >= 0 && !this.still) {
      // Shake: building up, then the lid flies.
      const s = phase(t, T.shake, T.lid)
      this.chest.rotation.z = s < 1 ? Math.sin(t * 46) * 0.035 * (1 - s * 0.4) : 0
      this.chest.position.x = s < 1 ? Math.sin(t * 61) * 0.025 : 0
      this.lid.rotation.x = -LID_OPEN * ease.outBack(phase(t, T.lid, 0.7))
      const light = ease.out(phase(t, T.light, 0.8))
      this.inner.material.opacity = light
      this.chestLight.intensity = light * 7
      const b = ease.out(phase(t, T.beams, 1.2))
      // A slow breath once it is up, so the light feels alive without flickering.
      this.beamMat.opacity = b * (0.7 + Math.sin(t * 5) * 0.05)
      this.beams.scale.set(1, Math.max(0.001, b), 1)
      this.updateSparks(t - T.sparks)

      // Camera: one arc down into the light.
      const c = phase(t, T.camera, T.end - T.camera)
      const k = ease.inOut(c)
      if (this.path) this.camera.position.copy(this.path.getPoint(k))
      target.lerp(new THREE.Vector3(0, 0.9, 0), ease.in(c))
      shift = baseShift * (1 - ease.out(c))
      this.camera.fov = baseFov + ease.in(c) * 18
      // The light swells as the camera reaches it (no tone mapping: the lights themselves brighten).
      this.chestLight.intensity = light * 7 * (1 + ease.in(c) * 4)
    } else if (t >= 0) {
      this.inner.material.opacity = 1
      this.chestLight.intensity = 7
    }

    this.camera.lookAt(target)
    this.camera.setViewOffset(w, h, 0, h * shift, w, h)
    this.camera.updateProjectionMatrix()
    this.renderer.render(this.scene, this.camera)
  }

  private updateSparks(t: number): void {
    const pos = this.sparks.geometry.getAttribute('position') as THREE.BufferAttribute
    const arr = pos.array as Float32Array
    for (let i = 0; i < arr.length / 3; i++) {
      const v = this.sparkVel.subarray(i * 4, i * 4 + 4)
      const life = t - v[3]
      if (life < 0) {
        arr[i * 3 + 1] = -50
        continue
      }
      arr[i * 3] = v[0] * life
      arr[i * 3 + 1] = 1.3 + v[1] * life - 2.2 * life * life
      arr[i * 3 + 2] = v[2] * life
    }
    pos.needsUpdate = true
    this.sparks.material.opacity = t > 0 ? Math.min(1, t * 3) : 0
  }
}
