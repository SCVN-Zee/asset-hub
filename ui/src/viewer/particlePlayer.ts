import * as THREE from 'three'
import type { MaterialLook, ParticleEmitter, ParticleSim, ParticleShape, SubEmitterType } from '@shared/types'
import { DEFAULT_STREAMS, evalMinMax, evalMinMaxColor, streamTexcoords } from './particleEval'

// Plays Unity particle systems in three.js, close to what Unity shows: the
// main module, emission (rate and bursts), shapes, gravity, velocity / force /
// limit velocity, colour, size and rotation over lifetime, texture sheets and
// noise, sub emitters and trails; billboards, stretched, horizontal, vertical
// and mesh particles. Simulated in
// Unity's local space of each emitter, mirrored along X like the prefab.

export interface EmitterLook {
  texture?: THREE.Texture
  blend: NonNullable<MaterialLook['blend']>
  /** Material colour (sRGB, 0–1). */
  tint: [number, number, number, number]
  /** Mesh particles: the mesh each particle is, scaled to Unity's import units. */
  mesh?: THREE.BufferGeometry
  /** The trails' own material, when the system draws trails. */
  trail?: Omit<EmitterLook, 'mesh' | 'trail'>
  /** A shader ported from Unity (viewer/shaderPorts): drawn by its own fragment shader instead of texture × tint. */
  custom?: CustomLook
  /** The project works in Gamma colour space: colours and textures are used as stored, as Unity does there. */
  gamma?: boolean
  /**
   * Unity's legacy particle shaders whose blend has no plain equivalent: "premultiply" (Alpha Blended Premultiply:
   * colour and alpha scaled by the particle's alpha) and "soft" (Additive (Soft): One, OneMinusSrcColor, which never burns out).
   */
  legacy?: 'premultiply' | 'soft'
}

interface StreamAttributes {
  s0: THREE.InstancedBufferAttribute
  s1: THREE.InstancedBufferAttribute
  layout: Array<{ stream: number; component: number }>
  uniforms: Record<string, THREE.IUniform>
}

export interface CustomLook {
  /** Reads `vUv` and `vColor` and the uniforms below, plus `uTime` (seconds since the effect started). */
  fragmentShader: string
  uniforms: Record<string, THREE.IUniform>
  /** three.js blend factors (Unity's `Blend [src] [dst]`). */
  blendSrc: THREE.BlendingSrcFactor
  blendDst: THREE.BlendingDstFactor
  /** Unity's ZWrite and Cull; off and both sides when not given. */
  depthWrite?: boolean
  side?: THREE.Side
}

const VERT = /* glsl */ `
attribute vec2 corner;
attribute vec3 iPos;
attribute vec3 iVel;
attribute vec4 iColor;
attribute vec3 iSRF;
uniform float uMode;
uniform mat3 uAlign;
attribute vec2 iRotXY;
attribute float iSizeY;
uniform vec2 uTiles;
uniform float uLength;
uniform float uVelScale;
uniform vec2 uPivot;
uniform float uSizeScale;
// Vertex streams for ported shaders: TEXCOORD0 and TEXCOORD1 per particle; uUvMap says which components are the vertex's UV (1 u, 2 v).
attribute vec4 iS0;
attribute vec4 iS1;
uniform vec4 uUvMap0;
uniform vec4 uUvMap1;
varying vec4 vTex0;
varying vec4 vTex1;
varying vec2 vUv;
varying vec2 vCorner;
varying vec4 vColor;
vec4 withUv(vec4 values, vec4 map, vec2 uv) {
  return mix(mix(values, vec4(uv.x), vec4(equal(map, vec4(1.0)))), vec4(uv.y), vec4(equal(map, vec4(2.0))));
}
void main() {
  float size = iSRF.x * uSizeScale;
  // Height apart from width when the system sizes per axis (a long thin ray).
  float sizeY = iSizeY * uSizeScale;
  float rot = iSRF.y;
  float frame = iSRF.z;
  vColor = iColor;
  vCorner = corner;
  // Sheet frames run left to right, top to bottom.
  float fx = mod(frame, uTiles.x);
  float fy = floor(frame / uTiles.x);
  vec2 uv = corner + 0.5;
  // Stretched, Unity lays the texture's U along the stretch, from the head (0) back to the tail (1).
  if (uMode > 0.5 && uMode < 1.5) uv = vec2(0.5 - corner.y, corner.x + 0.5);
  vUv = vec2((fx + uv.x) / uTiles.x, 1.0 - (fy + 1.0 - uv.y) / uTiles.y);
  vTex0 = withUv(iS0, uUvMap0, vUv);
  vTex1 = withUv(iS1, uUvMap1, vUv);
  float c = cos(rot);
  float s = sin(rot);
  vec2 q = corner * vec2(size, sizeY);
  vec2 p = vec2(c * q.x - s * q.y, s * q.x + c * q.y);
  if (uMode > 3.5) {
    // Facing the system's (Local) or the world's axes, turned by the particle's own 3D rotation: a ring on the ground.
    // Z is already in p; then X, then Y, like Unity.
    float cx = cos(iRotXY.x), sx = sin(iRotXY.x), cy = cos(iRotXY.y), sy = sin(iRotXY.y);
    mat3 rx = mat3(1.0, 0.0, 0.0, 0.0, cx, sx, 0.0, -sx, cx);
    mat3 ry = mat3(cy, 0.0, -sy, 0.0, 1.0, 0.0, sy, 0.0, cy);
    gl_Position = projectionMatrix * modelViewMatrix * vec4(iPos + uAlign * ry * rx * vec3(p, 0.0), 1.0);
    return;
  }
  if (uMode > 1.5 && uMode < 2.5) {
    // Horizontal: lying flat, facing up.
    gl_Position = projectionMatrix * modelViewMatrix * vec4(iPos + vec3(p.x, 0.0, p.y), 1.0);
    return;
  }
  vec4 mv = modelViewMatrix * vec4(iPos, 1.0);
  if (uMode > 0.5 && uMode < 1.5) {
    // Stretched along the velocity as seen on screen; the particle sits at the head.
    vec3 v = mat3(modelViewMatrix) * iVel;
    float speed = length(v.xy);
    vec2 dir = speed > 1e-5 ? v.xy / speed : vec2(0.0, 1.0);
    vec2 side = vec2(dir.y, -dir.x);
    float len = size * uLength + length(iVel) * uVelScale;
    // The pivot moves it along the stretch in lengths and sideways in sizes.
    mv.xy += side * (corner.x + uPivot.x) * size + dir * (corner.y - 0.5 + uPivot.y) * len;
  } else if (uMode > 2.5) {
    // Vertical: faces the camera but stays upright.
    mv.xy += vec2(corner.x * size, corner.y * sizeY);
  } else {
    mv.xy += p;
  }
  gl_Position = projectionMatrix * mv;
}
`

const FRAG = /* glsl */ `
uniform sampler2D map;
uniform float uHasMap;
uniform vec4 uTint;
uniform float uMultiply;
uniform float uPremultiply;
uniform float uLegacyPremultiply;
varying vec2 vUv;
varying vec2 vCorner;
varying vec4 vColor;
void main() {
  // Without a texture: a soft round dot, like Unity's default particle.
  // (smoothstep needs its edges in order: reversed ones are undefined, and draw a hard disc on some GPUs.)
  vec4 tex = uHasMap > 0.5 ? texture2D(map, vUv) : vec4(1.0, 1.0, 1.0, 1.0 - smoothstep(0.15, 0.5, length(vCorner)));
  vec4 c = tex * vColor * uTint;
  if (c.a < 0.003) discard;
  // Premultiplied blending (One, OneMinusSrcAlpha) expects colour already scaled by alpha, as Unity's shaders do.
  // Unity's legacy "Alpha Blended Premultiply" scales colour and alpha by the particle's alpha once more:
  // a texture's black, opaque background then barely covers what is behind.
  if (uLegacyPremultiply > 0.5) c *= vColor.a;
  else if (uPremultiply > 0.5) c.rgb *= c.a;
  gl_FragColor = uMultiply > 0.5 ? vec4(mix(vec3(1.0), c.rgb, c.a), 1.0) : c;
  // A Gamma project blends the values as they are; a Linear one is shown through sRGB.
  #ifndef GAMMA_SPACE
  #include <colorspace_fragment>
  #endif
}
`

/** Trail segments: quads from one recorded point to the next, facing the camera. */
const TRAIL_VERT = /* glsl */ `
attribute vec2 corner;
attribute vec3 iA;
attribute vec3 iB;
attribute vec2 iWidth;
attribute vec4 iColorA;
attribute vec4 iColorB;
attribute vec2 iU;
varying vec2 vUv;
varying vec4 vColor;
varying vec4 vTex0;
varying vec4 vTex1;
void main() {
  float along = corner.x + 0.5;
  vec4 a = modelViewMatrix * vec4(iA, 1.0);
  vec4 b = modelViewMatrix * vec4(iB, 1.0);
  vec2 d = b.xy - a.xy;
  vec2 dir = length(d) > 1e-6 ? normalize(d) : vec2(1.0, 0.0);
  vec2 side = vec2(-dir.y, dir.x);
  vec4 p = mix(a, b, along);
  p.xy += side * corner.y * mix(iWidth.x, iWidth.y, along);
  vColor = mix(iColorA, iColorB, along);
  vUv = vec2(mix(iU.x, iU.y, along), corner.y + 0.5);
  // Trails carry no per-particle streams: the UV, and zeros.
  vTex0 = vec4(vUv, 0.0, 0.0);
  vTex1 = vec4(0.0);
  gl_Position = projectionMatrix * p;
}
`

const TRAIL_FRAG = /* glsl */ `
uniform sampler2D map;
uniform float uHasMap;
uniform vec4 uTint;
uniform float uMultiply;
uniform float uPremultiply;
uniform float uLegacyPremultiply;
varying vec2 vUv;
varying vec4 vColor;
void main() {
  vec4 tex = uHasMap > 0.5 ? texture2D(map, vUv) : vec4(1.0);
  vec4 c = tex * vColor * uTint;
  if (c.a < 0.003) discard;
  // Premultiplied blending (One, OneMinusSrcAlpha) expects colour already scaled by alpha, as Unity's shaders do.
  // Unity's legacy "Alpha Blended Premultiply" scales colour and alpha by the particle's alpha once more:
  // a texture's black, opaque background then barely covers what is behind.
  if (uLegacyPremultiply > 0.5) c *= vColor.a;
  else if (uPremultiply > 0.5) c.rgb *= c.a;
  gl_FragColor = uMultiply > 0.5 ? vec4(mix(vec3(1.0), c.rgb, c.a), 1.0) : c;
  // A Gamma project blends the values as they are; a Linear one is shown through sRGB.
  #ifndef GAMMA_SPACE
  #include <colorspace_fragment>
  #endif
}
`

/** A mesh's UV moved into the texture sheet's frame (left to right, top to bottom), as Unity does for mesh particles. */
const SHEET_UV = /* glsl */ `
attribute float instanceFrame;
uniform vec2 uTiles;
vec2 sheetUv(vec2 uv) {
  float fx = mod(instanceFrame, uTiles.x);
  float fy = floor(instanceFrame / uTiles.x);
  return vec2((fx + uv.x) / uTiles.x, 1.0 - (fy + 1.0 - uv.y) / uTiles.y);
}
`

/** Mesh particles drawn by a ported shader: the mesh's UV where the streams put UVs, the particle's streams (custom data…), colour and alpha. */
const MESH_VERT = /* glsl */ `
${SHEET_UV}
attribute float instanceAlpha;
attribute vec4 iS0;
attribute vec4 iS1;
uniform vec4 uUvMap0;
uniform vec4 uUvMap1;
varying vec4 vTex0;
varying vec4 vTex1;
varying vec4 vColor;
varying vec3 vNormalW;
varying vec3 vViewDirW;
vec4 withUv(vec4 values, vec4 map, vec2 uv) {
  return mix(mix(values, vec4(uv.x), vec4(equal(map, vec4(1.0)))), vec4(uv.y), vec4(equal(map, vec4(2.0))));
}
void main() {
  vec2 tileUv = sheetUv(uv);
  vTex0 = withUv(iS0, uUvMap0, tileUv);
  vTex1 = withUv(iS1, uUvMap1, tileUv);
  vec3 c = vec3(1.0);
  #ifdef USE_INSTANCING_COLOR
  c = instanceColor;
  #endif
  vColor = vec4(c, instanceAlpha);
  // World-space normal and direction to the camera, for the shaders' rim and backface tint.
  mat4 world = modelMatrix * instanceMatrix;
  vec4 at = world * vec4(position, 1.0);
  vNormalW = mat3(world) * normal;
  vViewDirW = cameraPosition - at.xyz;
  gl_Position = projectionMatrix * viewMatrix * at;
}
`

/** Points kept per trail; older ones drop off the end. */
const TRAIL_POINTS = 10
/** Particles with trails, at most (trails cost TRAIL_POINTS quads each). */
const MAX_TRAILS = 600

const DEG = Math.PI / 180
const GRAVITY = 9.81
const toLinear = (v: number) => Math.pow(v, 2.2)

/** Longest an emitter is played ahead for prewarm (seconds): enough for any particle lifetime worth showing. */
const PREWARM_MAX = 12
/** One frame of the timeline and of runs done without drawing: a 30th of a second. */
export const FRAME = 1 / 30

/**
 * The particles' random numbers (mulberry32): seeded at each play of an effect,
 * so going back on the timeline plays the same particles again.
 */
let rngState = 1
function rnd(): number {
  let t = (rngState = (rngState + 0x6d2b79f5) | 0)
  t = Math.imul(t ^ (t >>> 15), t | 1)
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296
}
const newSeed = () => Math.floor(Math.random() * 4294967296)

/** A quad, shared by every emitter. */
function quad(): THREE.InstancedBufferGeometry {
  const g = new THREE.InstancedBufferGeometry()
  g.setAttribute('corner', new THREE.BufferAttribute(new Float32Array([-0.5, -0.5, 0.5, -0.5, 0.5, 0.5, -0.5, 0.5]), 2))
  // Also a "position" so three.js knows how many vertices there are.
  g.setAttribute('position', new THREE.BufferAttribute(new Float32Array(12), 3))
  g.setIndex([0, 1, 2, 0, 2, 3])
  return g
}

function blending(material: THREE.ShaderMaterial, blend: EmitterLook['blend']) {
  if (blend === 'additive') material.blending = THREE.AdditiveBlending
  else if (blend === 'multiply') {
    material.blending = THREE.CustomBlending
    material.blendSrc = THREE.DstColorFactor
    material.blendDst = THREE.ZeroFactor
  } else if (blend === 'premultiply') {
    material.blending = THREE.CustomBlending
    material.blendSrc = THREE.OneFactor
    material.blendDst = THREE.OneMinusSrcAlphaFactor
  } else material.blending = THREE.NormalBlending
}

/** Unity's Euler order (Z, then X, then Y) for the shape's rotation. */
const shapeRotation = (s: ParticleShape) => new THREE.Quaternion().setFromEuler(new THREE.Euler(s.rotation[0] * DEG, s.rotation[1] * DEG, s.rotation[2] * DEG, 'YXZ'))

const randomUnit = (out: THREE.Vector3) => {
  const z = rnd() * 2 - 1
  const a = rnd() * Math.PI * 2
  const r = Math.sqrt(1 - z * z)
  return out.set(r * Math.cos(a), r * Math.sin(a), z)
}

/** Where a particle starts and where it heads, in the emitter's (Unity) space. */
function emitFromShape(shape: ParticleShape | undefined, pos: THREE.Vector3, dir: THREE.Vector3, rot: THREE.Quaternion | undefined): void {
  if (!shape) {
    // Shape module off: from the emitter's origin, along +Z.
    pos.set(0, 0, 0)
    dir.set(0, 0, 1)
    return
  }
  const arc = (shape.arc / 360) * Math.PI * 2
  const radial = () => shape.radius * (1 - shape.radiusThickness * (1 - Math.cbrt(rnd())))
  switch (shape.type) {
    case 0:
    case 1:
    case 2:
    case 3: {
      randomUnit(dir)
      if (shape.type >= 2 && dir.z < 0) dir.z = -dir.z
      pos.copy(dir).multiplyScalar(shape.type === 1 || shape.type === 3 ? shape.radius : radial())
      break
    }
    case 4:
    case 7:
    case 8:
    case 9: {
      // Cone: from a disc of `radius`, spreading out by `angle` towards the rim.
      const a = rnd() * arc
      const d = shape.type === 7 || shape.type === 9 ? 1 : Math.sqrt(1 - shape.radiusThickness * rnd())
      const x = Math.cos(a) * d
      const y = Math.sin(a) * d
      const spread = Math.sin(shape.angle * DEG)
      pos.set(x * shape.radius, y * shape.radius, 0)
      dir.set(x * spread, y * spread, Math.cos(shape.angle * DEG)).normalize()
      // Volume cones emit along their length too.
      if (shape.type >= 8) pos.addScaledVector(dir, rnd() * shape.length)
      break
    }
    case 5:
    case 15:
    case 16:
      pos.set(rnd() - 0.5, rnd() - 0.5, rnd() - 0.5)
      dir.set(0, 0, 1)
      break
    case 10:
    case 11: {
      const a = rnd() * arc
      const r = shape.type === 11 ? shape.radius : radial()
      pos.set(Math.cos(a) * r, Math.sin(a) * r, 0)
      dir.set(Math.cos(a), Math.sin(a), 0)
      break
    }
    case 12:
      pos.set((rnd() - 0.5) * 2 * shape.radius, 0, 0)
      dir.set(0, 1, 0)
      break
    case 18:
      pos.set(rnd() - 0.5, rnd() - 0.5, 0)
      dir.set(0, 0, 1)
      break
    default:
      // Meshes, sprites, donuts: a sphere of the shape's size is close enough to see the effect.
      randomUnit(dir)
      pos.copy(dir).multiplyScalar(radial() || 0.5)
  }
  if (shape.randomDirection > 0) dir.lerp(randomUnit(new THREE.Vector3()), shape.randomDirection).normalize()
  pos.multiply(new THREE.Vector3(...shape.scale))
  if (rot) {
    pos.applyQuaternion(rot)
    dir.applyQuaternion(rot)
  }
  pos.add(new THREE.Vector3(...shape.position))
}

/** Cheap smooth noise: sums of sines, enough to make particles wander like Unity's noise module. */
const wander = (x: number, y: number, z: number, t: number) => [
  Math.sin(y * 1.7 + t * 1.3) + Math.sin(z * 2.3 - t),
  Math.sin(z * 1.9 + t * 1.1) + Math.sin(x * 2.1 + t * 0.7),
  Math.sin(x * 1.3 - t * 1.2) + Math.sin(y * 2.7 + t * 0.9)
]

function shaderMaterial(vertexShader: string, fragmentShader: string, look: Omit<EmitterLook, 'mesh' | 'trail'>, extra: Record<string, THREE.IUniform> = {}, defines: Record<string, string> = {}): THREE.ShaderMaterial {
  const custom = look.custom
  if (custom) {
    const material = new THREE.ShaderMaterial({
      vertexShader,
      fragmentShader: custom.fragmentShader,
      // Each emitter keeps its own clock; values (textures too) are shared, as they never change.
      uniforms: { ...Object.fromEntries(Object.entries(custom.uniforms).map(([k, u]) => [k, { value: u.value }])), uTime: { value: 0 }, ...extra },
      // A material that writes depth and replaces what is behind it (Unity's opaque and cutout) is drawn first, as Unity's queue does.
      transparent: !(custom.depthWrite && custom.blendDst === THREE.ZeroFactor),
      depthWrite: custom.depthWrite ?? false,
      side: custom.side ?? THREE.DoubleSide,
      blending: THREE.CustomBlending,
      blendSrc: custom.blendSrc,
      blendDst: custom.blendDst,
      defines: { ...(look.gamma ? { GAMMA_SPACE: '' } : {}), ...defines }
    })
    return material
  }
  const material = new THREE.ShaderMaterial({
    vertexShader,
    fragmentShader,
    uniforms: {
      map: { value: look.texture ?? null },
      uHasMap: { value: look.texture ? 1 : 0 },
      uTint: { value: look.gamma ? new THREE.Vector4(...look.tint) : new THREE.Vector4(toLinear(look.tint[0]), toLinear(look.tint[1]), toLinear(look.tint[2]), look.tint[3]) },
      uMultiply: { value: look.blend === 'multiply' ? 1 : 0 },
      uPremultiply: { value: look.blend === 'premultiply' || look.legacy === 'soft' ? 1 : 0 },
      uLegacyPremultiply: { value: look.legacy === 'premultiply' ? 1 : 0 },
      ...extra
    },
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
    defines: look.gamma ? { GAMMA_SPACE: '' } : {}
  })
  blending(material, look.blend)
  if (look.legacy === 'soft') {
    material.blending = THREE.CustomBlending
    material.blendSrc = THREE.OneFactor
    material.blendDst = THREE.OneMinusSrcColorFactor
  }
  return material
}

/** A sub emitter link: the system to start, and when. */
interface SubLink {
  player: EmitterPlayer
  type: SubEmitterType
  inherit: number
  probability: number
}

const scratch = new THREE.Vector3()

/** One particle system, drawn under the object of its GameObject: instanced quads, or instanced meshes. */
export class EmitterPlayer {
  /** What is added to the scene: quads or meshes, plus trails. */
  readonly mesh: THREE.Object3D
  /** Started by another system's particles (a sub emitter): emits nothing on its own. */
  isSub = false
  readonly subs: SubLink[] = []
  private readonly sim: ParticleSim
  private readonly max: number
  private readonly shapeRot?: THREE.Quaternion
  private readonly quads?: { geometry: THREE.InstancedBufferGeometry; material: THREE.ShaderMaterial; pos: THREE.InstancedBufferAttribute; vel: THREE.InstancedBufferAttribute; color: THREE.InstancedBufferAttribute; srf: THREE.InstancedBufferAttribute; rotXY: THREE.InstancedBufferAttribute; sizeY: THREE.InstancedBufferAttribute; streams?: StreamAttributes }
  private readonly meshes?: THREE.InstancedMesh
  /** Vertex streams of mesh particles drawn by a ported shader. */
  private readonly meshStreams?: StreamAttributes
  /** How far a mesh particle of size 1 spans (its mesh's bounds around the pivot, across): 1 for billboards. */
  private readonly span: number = 1
  private readonly meshAlpha?: THREE.InstancedBufferAttribute
  /** Texture sheet frame of each mesh particle. */
  private readonly meshFrame?: THREE.InstancedBufferAttribute
  private readonly trail?: { geometry: THREE.InstancedBufferGeometry; material: THREE.ShaderMaterial; attrs: Record<'a' | 'b' | 'width' | 'colorA' | 'colorB' | 'u', THREE.InstancedBufferAttribute>; hist: Float32Array; histTime: Float32Array; histCount: Uint8Array; has: Uint8Array }
  // Per particle.
  private readonly pos: Float32Array
  private readonly vel: Float32Array
  private readonly age: Float32Array
  private readonly life: Float32Array
  private readonly size: Float32Array
  private readonly rot: Float32Array
  /** Start height and depth (equal to `size` unless the system sizes per axis). */
  private readonly sizeYZ: Float32Array
  /** Angles around X and Y (3D rotation), mirrored like the rest. */
  private readonly rotXY: Float32Array
  /** World alignment: undoes the system's own rotation, so quads follow the world's axes. */
  private readonly align?: THREE.Matrix4
  private readonly rand: Float32Array
  private readonly color: Float32Array
  private count = 0
  private time = 0
  private delay = 0
  private emitDebt = 0
  private lastCycle = -1
  private lastCycleTime = -1e-6

  constructor(
    /** `prefab` names the nested prefab it comes from (absent for the opened one). */
    readonly emitter: ParticleEmitter & { prefab?: string },
    private readonly look: EmitterLook,
    /** Plays one cycle only, even when the system loops (a shot fired on top of the effect). */
    readonly once = false
  ) {
    this.sim = emitter.sim
    // Enough for the effect without reserving Unity's default 1000 for a 20-particle sparkle.
    this.max = Math.max(1, Math.min(this.sim.maxParticles, Math.max(64, emitter.peak * 2), 5000))
    const n = this.max
    this.pos = new Float32Array(n * 3)
    this.vel = new Float32Array(n * 3)
    this.age = new Float32Array(n)
    this.life = new Float32Array(n)
    this.size = new Float32Array(n)
    this.rot = new Float32Array(n * 2)
    this.rotXY = new Float32Array(n * 2)
    this.sizeYZ = new Float32Array(n * 2)
    if (this.sim.alignment === 'world') this.align = new THREE.Matrix4()
    this.rand = new Float32Array(n * 4)
    this.color = new Float32Array(n * 4)
    if (this.sim.shape && this.sim.shape.rotation.some((v) => v !== 0)) this.shapeRot = shapeRotation(this.sim.shape)

    const group = new THREE.Group()
    group.userData.particles = true
    // Unity's local space mirrored along X, like the prefab's transforms.
    group.scale.set(-1, 1, 1)
    group.visible = emitter.renderMode !== 'none'
    this.mesh = group

    if (emitter.renderMode === 'mesh' && look.mesh) {
      // Framing counts the mesh itself: a slash mesh can be many times its particle's size.
      if (!look.mesh.boundingSphere) look.mesh.computeBoundingSphere()
      const b = look.mesh.boundingSphere
      if (b && Number.isFinite(b.radius) && b.radius > 0) this.span = 2 * (b.center.length() + b.radius)
    }
    if (emitter.renderMode === 'mesh' && look.mesh && look.custom) {
      const geometry = look.mesh.clone()
      this.meshAlpha = new THREE.InstancedBufferAttribute(new Float32Array(n).fill(1), 1).setUsage(THREE.DynamicDrawUsage)
      geometry.setAttribute('instanceAlpha', this.meshAlpha)
      this.meshFrame = new THREE.InstancedBufferAttribute(new Float32Array(n), 1).setUsage(THREE.DynamicDrawUsage)
      geometry.setAttribute('instanceFrame', this.meshFrame)
      if (!geometry.getAttribute('uv')) geometry.setAttribute('uv', new THREE.BufferAttribute(new Float32Array(geometry.getAttribute('position').count * 2), 2))
      if (!geometry.getAttribute('normal')) geometry.computeVertexNormals()
      const streams = this.streamAttributes(n)
      geometry.setAttribute('iS0', streams.s0)
      geometry.setAttribute('iS1', streams.s1)
      this.meshStreams = streams
      this.meshes = new THREE.InstancedMesh(geometry, shaderMaterial(MESH_VERT, '', look, { ...streams.uniforms, uTiles: { value: this.tiles() } }, { MESH_PARTICLE: '' }), n)
      this.meshes.instanceMatrix.setUsage(THREE.DynamicDrawUsage)
      this.meshes.setColorAt(0, new THREE.Color(1, 1, 1))
      this.meshes.count = 0
      this.meshes.frustumCulled = false
      this.meshes.userData.particles = true
      group.add(this.meshes)
    } else if (emitter.renderMode === 'mesh' && look.mesh) {
      const material = new THREE.MeshBasicMaterial({ map: look.texture ?? null, transparent: true, depthWrite: false, side: THREE.DoubleSide })
      const c = look.tint
      material.color.setRGB(c[0], c[1], c[2], THREE.SRGBColorSpace)
      material.opacity = c[3]
      if (look.blend === 'additive') material.blending = THREE.AdditiveBlending
      else if (look.blend === 'multiply') material.blending = THREE.MultiplyBlending
      else if (look.blend === 'premultiply') {
        // three.js scales the colour by alpha itself when told the output is premultiplied.
        material.premultipliedAlpha = true
        material.blending = THREE.CustomBlending
        material.blendSrc = THREE.OneFactor
        material.blendDst = THREE.OneMinusSrcAlphaFactor
      }
      // Each particle's own alpha (fading, a see-through shield), as three.js instances only carry a colour.
      const geometry = look.mesh.clone()
      this.meshAlpha = new THREE.InstancedBufferAttribute(new Float32Array(n).fill(1), 1).setUsage(THREE.DynamicDrawUsage)
      geometry.setAttribute('instanceAlpha', this.meshAlpha)
      this.meshFrame = new THREE.InstancedBufferAttribute(new Float32Array(n), 1).setUsage(THREE.DynamicDrawUsage)
      geometry.setAttribute('instanceFrame', this.meshFrame)
      const tiles = this.tiles()
      material.onBeforeCompile = (shader) => {
        shader.uniforms.uTiles = { value: tiles }
        shader.vertexShader = shader.vertexShader
          .replace('#include <common>', `#include <common>\nattribute float instanceAlpha;\nvarying float vInstanceAlpha;\n${SHEET_UV}`)
          .replace('#include <uv_vertex>', '#include <uv_vertex>\n#ifdef USE_MAP\nvMapUv = sheetUv(vMapUv);\n#endif')
          .replace('#include <begin_vertex>', '#include <begin_vertex>\nvInstanceAlpha = instanceAlpha;')
        shader.fragmentShader = shader.fragmentShader
          .replace('#include <common>', '#include <common>\nvarying float vInstanceAlpha;')
          .replace('#include <color_fragment>', '#include <color_fragment>\ndiffuseColor.a *= vInstanceAlpha;')
      }
      this.meshes = new THREE.InstancedMesh(geometry, material, n)
      this.meshes.instanceMatrix.setUsage(THREE.DynamicDrawUsage)
      this.meshes.setColorAt(0, new THREE.Color(1, 1, 1))
      this.meshes.count = 0
      this.meshes.frustumCulled = false
      this.meshes.userData.particles = true
      group.add(this.meshes)
    } else {
      const geometry = quad()
      const attr = (size: number) => new THREE.InstancedBufferAttribute(new Float32Array(n * size), size).setUsage(THREE.DynamicDrawUsage)
      const q = { geometry, pos: attr(3), vel: attr(3), color: attr(4), srf: attr(3), rotXY: attr(2), sizeY: attr(1) }
      geometry.setAttribute('iRotXY', q.rotXY)
      geometry.setAttribute('iSizeY', q.sizeY)
      geometry.setAttribute('iPos', q.pos)
      geometry.setAttribute('iVel', q.vel)
      geometry.setAttribute('iColor', q.color)
      geometry.setAttribute('iSRF', q.srf)
      geometry.instanceCount = 0
      // Ported shaders read the renderer's vertex streams (custom data, randoms…), in Unity's TEXCOORD layout.
      const streams = look.custom ? this.streamAttributes(n) : undefined
      if (streams) {
        geometry.setAttribute('iS0', streams.s0)
        geometry.setAttribute('iS1', streams.s1)
      }
      // Billboards aligned to the system's axes, not the camera, are drawn as quads in its space.
      const oriented = emitter.renderMode === 'billboard' && (this.sim.alignment === 'local' || this.sim.alignment === 'world')
      const mode = oriented ? 4 : { billboard: 0, mesh: 0, none: 0, stretched: 1, horizontal: 2, vertical: 3 }[emitter.renderMode]
      const material = shaderMaterial(VERT, FRAG, look, {
        uMode: { value: mode },
        uAlign: { value: new THREE.Matrix3() },
        uTiles: { value: this.tiles() },
        uLength: { value: this.sim.lengthScale },
        uVelScale: { value: this.sim.velocityScale },
        uPivot: { value: new THREE.Vector2(this.sim.pivot[0], this.sim.pivot[1]) },
        // Unity scales particles with their transforms (hierarchy scaling): kept up to date in update().
        uSizeScale: { value: 1 },
        ...streams?.uniforms
      })
      this.quads = { ...q, material, streams }
      const m = new THREE.Mesh(geometry, material)
      m.frustumCulled = false
      m.userData.particles = true
      group.add(m)
    }

    if (this.sim.trail) {
      const k = Math.min(n, MAX_TRAILS)
      const segs = k * (TRAIL_POINTS - 1)
      const geometry = quad()
      const attr = (size: number) => new THREE.InstancedBufferAttribute(new Float32Array(segs * size), size).setUsage(THREE.DynamicDrawUsage)
      const attrs = { a: attr(3), b: attr(3), width: attr(2), colorA: attr(4), colorB: attr(4), u: attr(2) }
      geometry.setAttribute('iA', attrs.a)
      geometry.setAttribute('iB', attrs.b)
      geometry.setAttribute('iWidth', attrs.width)
      geometry.setAttribute('iColorA', attrs.colorA)
      geometry.setAttribute('iColorB', attrs.colorB)
      geometry.setAttribute('iU', attrs.u)
      geometry.instanceCount = 0
      const material = shaderMaterial(TRAIL_VERT, TRAIL_FRAG, look.trail ?? { blend: look.blend, tint: [1, 1, 1, 1] })
      this.trail = { geometry, material, attrs, hist: new Float32Array(n * TRAIL_POINTS * 3), histTime: new Float32Array(n * TRAIL_POINTS), histCount: new Uint8Array(n), has: new Uint8Array(n) }
      const m = new THREE.Mesh(geometry, material)
      m.frustumCulled = false
      m.userData.particles = true
      group.add(m)
    }
    // Unity sorts an effect's renderers by their order in layer, then by distance shifted by the fudge: emitters
    // in one spot (a dark backdrop behind a glow) keep the order the effect was made with.
    const order = this.sim.sortingOrder * 1e6 - this.sim.sortingFudge
    for (const child of group.children) child.renderOrder = order
    this.restart()
  }

  /** Loops for good: a shot (`once`) plays its first cycle only. */
  private get looping(): boolean {
    return this.sim.looping && !this.once
  }

  /** Another player of the same system and look (shared textures and meshes), playing one cycle. */
  shot(): EmitterPlayer {
    return new EmitterPlayer(this.emitter, this.look, true)
  }

  /** Starts over: a new delay, no particles; prewarmed loops start full. */
  restart(): void {
    this.count = 0
    this.time = 0
    this.emitDebt = 0
    this.lastCycle = -1
    this.lastCycleTime = -1e-6
    this.delay = evalMinMax(this.sim.startDelay, 0, rnd())
    if (this.sim.prewarm && this.looping && !this.isSub) {
      this.delay = 0
      // Unity plays one whole cycle ahead; what is on screen afterwards are the particles of its last
      // lifetime, so that much (and a little more) looks the same. Some effects loop over 100000 s:
      // a whole cycle was 3 million steps, five seconds and more each time the effect started over.
      const warm = Math.min(this.sim.duration, Math.max(1, this.emitter.lifetime * 1.5), PREWARM_MAX)
      for (let t = 0; t < warm; t += 1 / 30) this.step(1 / 30)
    }
    this.upload()
  }

  /** Done for good: a one-shot past its duration with nothing left alive (sub emitters: nothing alive). */
  get finished(): boolean {
    if (this.isSub) return this.count === 0
    return !this.looping && this.time > this.delay + this.sim.duration && this.count === 0
  }

  get alive(): number {
    return this.count
  }

  update(dt: number): void {
    this.step(dt * this.sim.simulationSpeed)
    for (const m of [this.quads?.material, this.trail?.material, this.meshes?.material as THREE.ShaderMaterial | undefined]) if (m?.uniforms?.uTime) m.uniforms.uTime.value = this.time
    if (this.quads && this.mesh.parent) {
      const s = this.mesh.parent.getWorldScale(scratch)
      this.quads.material.uniforms.uSizeScale.value = (Math.abs(s.x) + Math.abs(s.y) + Math.abs(s.z)) / 3
    }
    if (this.align && this.mesh.parent) {
      // In the mirrored space the particles live in: mirror · (system's rotation)⁻¹ · mirror.
      const mirror = new THREE.Matrix4().makeScale(-1, 1, 1)
      const undo = new THREE.Matrix4().makeRotationFromQuaternion(this.mesh.parent.getWorldQuaternion(new THREE.Quaternion()).invert())
      this.align.copy(mirror).multiply(undo).multiply(mirror)
      this.quads?.material.uniforms.uAlign.value.setFromMatrix4(this.align)
    }
    this.upload()
  }

  /** Each live particle's position (in the prefab's space) and size, for framing the camera on where they really go. */
  sample(visit: (at: THREE.Vector3, size: number) => void): void {
    if (this.emitter.renderMode === 'none' || !this.mesh.parent || !this.count) return
    this.mesh.updateWorldMatrix(true, false)
    const s = this.mesh.parent.getWorldScale(scratch)
    const k = (Math.abs(s.x) + Math.abs(s.y) + Math.abs(s.z)) / 3
    const at = new THREE.Vector3()
    for (let i = 0; i < this.count; i++) visit(this.mesh.localToWorld(at.set(this.pos[i * 3], this.pos[i * 3 + 1], this.pos[i * 3 + 2])), this.size[i] * k * this.span)
  }

  /** Space particles may reach, in the emitter's space, to frame the camera. */
  reach(): number {
    const s = this.sim
    const life = evalMinMax(s.startLifetime, 0, 1)
    const speed = Math.abs(evalMinMax(s.startSpeed, 0, 1))
    const fall = 0.5 * GRAVITY * Math.abs(evalMinMax(s.gravity, 0, 1)) * life * life
    const shape = s.shape ? Math.max(s.shape.radius, ...s.shape.scale.map((v) => v / 2)) + (s.shape.type >= 8 && s.shape.type <= 9 ? s.shape.length : 0) : 0
    return Math.min(50, shape + Math.max(speed * life, fall) + evalMinMax(s.startSize, 0, 1))
  }

  /**
   * Started by a parent particle at `at` (this system's space): what Unity's
   * sub emitter emits — its bursts, or a second of its rate when it has none.
   */
  trigger(at: THREE.Vector3, inherit: number, parent?: { color: ArrayLike<number>; size: number }): void {
    const s = this.sim
    const bursts = s.bursts.reduce((n, b) => n + Math.round(evalMinMax(b.count, 0, rnd())) * Math.max(1, b.cycles), 0)
    const count = Math.min(60, bursts || Math.round(evalMinMax(s.rate, 0, rnd()) * Math.min(1, s.duration)))
    for (let i = 0; i < count; i++) this.spawn(0, at, inherit, parent)
  }

  /** Birth sub emitters: emitting from a live parent particle, at this system's rate. */
  emitFrom(at: THREE.Vector3, dt: number, inherit: number, parent?: { color: ArrayLike<number>; size: number }): void {
    const n = Math.floor(Math.max(0, evalMinMax(this.sim.rate, 0, rnd())) * dt + rnd())
    for (let i = 0; i < n; i++) this.spawn(0, at, inherit, parent)
  }

  private step(dt: number): void {
    if (dt <= 0) return
    this.time += dt
    if (!this.isSub) this.emit(dt)
    this.move(dt)
  }

  private emit(dt: number): void {
    const s = this.sim
    const local = this.time - this.delay
    if (local < 0) return
    const looping = this.looping
    if (!looping && local > s.duration + dt) return
    const cycle = looping ? Math.floor(local / s.duration) : 0
    const ct = looping ? local - cycle * s.duration : Math.min(local, s.duration)
    const from = cycle !== this.lastCycle ? -1e-6 : this.lastCycleTime
    this.lastCycle = cycle
    this.lastCycleTime = ct
    const nt = s.duration > 0 ? ct / s.duration : 0
    this.emitDebt += Math.max(0, evalMinMax(s.rate, nt, rnd())) * dt
    const n = Math.floor(this.emitDebt)
    this.emitDebt -= n
    for (let i = 0; i < n; i++) this.spawn(nt)
    for (const b of s.bursts) {
      // Infinite cycles (0) repeat until the end of the system's duration.
      const cycles = b.cycles > 0 ? b.cycles : Math.ceil((s.duration - b.time) / b.interval)
      for (let k = 0; k < cycles; k++) {
        const at = b.time + k * b.interval
        if (at > from && at <= ct && rnd() <= b.probability) {
          const count = Math.round(evalMinMax(b.count, nt, rnd()))
          for (let i = 0; i < count; i++) this.spawn(nt)
        }
      }
    }
  }

  private spawn(nt: number, at?: THREE.Vector3, inherit = 0, parent?: { color: ArrayLike<number>; size: number }): void {
    if (this.count >= this.max) return
    const s = this.sim
    const i = this.count++
    const p = new THREE.Vector3()
    const d = new THREE.Vector3()
    emitFromShape(s.shape, p, d, this.shapeRot)
    if (at) p.add(at)
    const speed = evalMinMax(s.startSpeed, nt, rnd())
    this.pos.set([p.x, p.y, p.z], i * 3)
    this.vel.set([d.x * speed, d.y * speed, d.z * speed], i * 3)
    this.age[i] = 0
    this.life[i] = Math.max(0.01, evalMinMax(s.startLifetime, nt, rnd()))
    const inherited = parent && inherit & 2 ? parent.size : 1
    this.size[i] = evalMinMax(s.startSize, nt, rnd()) * inherited
    const [sy, sz] = s.startSize3D ?? []
    this.sizeYZ[i * 2] = sy ? evalMinMax(sy, nt, rnd()) * inherited : this.size[i]
    this.sizeYZ[i * 2 + 1] = sz ? evalMinMax(sz, nt, rnd()) * inherited : this.size[i]
    // Unity turns clockwise for a positive angle; mirrored here.
    this.rot[i * 2] = -evalMinMax(s.startRotation, nt, rnd())
    this.rot[i * 2 + 1] = 0
    // X keeps its sign under the X mirror; Y turns the other way, like Z.
    const [rx, ry] = s.startRotation3D ?? []
    this.rotXY[i * 2] = rx ? evalMinMax(rx, nt, rnd()) : 0
    this.rotXY[i * 2 + 1] = ry ? -evalMinMax(ry, nt, rnd()) : 0
    for (let k = 0; k < 4; k++) this.rand[i * 4 + k] = rnd()
    const c = evalMinMaxColor(s.startColor, nt, rnd())
    if (parent && inherit & 1) for (let k = 0; k < 4; k++) c[k] *= parent.color[k]
    this.color.set(c, i * 4)
    if (this.trail) {
      this.trail.has[i] = rnd() < (s.trail?.ratio ?? 1) && i < MAX_TRAILS ? 1 : 0
      this.trail.histCount[i] = 0
    }
    for (const sub of this.subs) if (sub.type === 'birth' && rnd() <= sub.probability) this.fire(sub, i, true)
  }

  /** Starts a sub emitter at particle `i`, bringing its position into the sub emitter's space. */
  private fire(sub: SubLink, i: number, burstOnly = false, dt = 0): void {
    const target = sub.player.mesh
    if (!target.parent || !this.mesh.parent) return
    const at = new THREE.Vector3(this.pos[i * 3], this.pos[i * 3 + 1], this.pos[i * 3 + 2])
    this.mesh.localToWorld(at)
    target.worldToLocal(at)
    const parent = { color: this.color.subarray(i * 4, i * 4 + 4), size: this.size[i] }
    if (burstOnly) {
      // Birth: the sub emitter's bursts when the particle appears; its rate while it lives (below).
      if (sub.player.sim.bursts.length) sub.player.trigger(at, sub.inherit, parent)
    } else if (dt) sub.player.emitFrom(at, dt, sub.inherit, parent)
    else sub.player.trigger(at, sub.inherit, parent)
  }

  private move(dt: number): void {
    const s = this.sim
    const nt = s.duration > 0 ? ((this.time - this.delay) % s.duration) / s.duration : 0
    if (this.subs.length) this.mesh.updateWorldMatrix(true, false)
    const births = this.subs.filter((sub) => sub.type === 'birth' && sub.player.sim.rate.scalar > 0)
    const deaths = this.subs.filter((sub) => sub.type === 'death' || sub.type === 'collision')
    // Gravity pulls down in the world; brought into the emitter's space.
    const g = new THREE.Vector3(0, -GRAVITY * evalMinMax(s.gravity, nt, 0.5), 0)
    if (g.y !== 0 && this.mesh.parent) {
      const q = this.mesh.getWorldQuaternion(new THREE.Quaternion()).invert()
      g.applyQuaternion(q)
      g.x = -g.x
    }
    for (let i = 0; i < this.count; ) {
      this.age[i] += dt
      if (this.age[i] >= this.life[i]) {
        // Collision sub emitters fire at death too: the preview has nothing to collide with.
        for (const sub of deaths) if (rnd() <= sub.probability) this.fire(sub, i)
        this.remove(i)
        continue
      }
      const t = this.age[i] / this.life[i]
      const r = (k: number) => this.rand[i * 4 + k]
      let vx = this.vel[i * 3] + g.x * dt
      let vy = this.vel[i * 3 + 1] + g.y * dt
      let vz = this.vel[i * 3 + 2] + g.z * dt
      if (s.force) {
        vx += evalMinMax(s.force[0], t, r(0)) * dt
        vy += evalMinMax(s.force[1], t, r(1)) * dt
        vz += evalMinMax(s.force[2], t, r(2)) * dt
      }
      if (s.limit) {
        const lim = evalMinMax(s.limit.limit, t, r(3))
        const sp = Math.hypot(vx, vy, vz)
        if (sp > lim && sp > 0) {
          const k = 1 - s.limit.dampen * (1 - lim / sp)
          vx *= k
          vy *= k
          vz *= k
        }
      }
      this.vel[i * 3] = vx
      this.vel[i * 3 + 1] = vy
      this.vel[i * 3 + 2] = vz
      // Velocity over lifetime moves particles without changing their own velocity.
      let ox = 0
      let oy = 0
      let oz = 0
      if (s.velocity) {
        ox = evalMinMax(s.velocity[0], t, r(0))
        oy = evalMinMax(s.velocity[1], t, r(1))
        oz = evalMinMax(s.velocity[2], t, r(2))
      }
      if (s.noise) {
        const k = evalMinMax(s.noise.strength, t, r(3))
        const f = s.noise.frequency
        const w = wander(this.pos[i * 3] * f + r(0) * 9, this.pos[i * 3 + 1] * f + r(1) * 9, this.pos[i * 3 + 2] * f, this.time * (1 + s.noise.scrollSpeed))
        ox += w[0] * k
        oy += w[1] * k
        oz += w[2] * k
      }
      this.pos[i * 3] += (vx + ox) * dt
      this.pos[i * 3 + 1] += (vy + oy) * dt
      this.pos[i * 3 + 2] += (vz + oz) * dt
      if (s.rotationOverLifetime) this.rot[i * 2 + 1] -= evalMinMax(s.rotationOverLifetime, t, r(2)) * dt
      if (s.rotationOverLifetime3D) {
        this.rotXY[i * 2] += evalMinMax(s.rotationOverLifetime3D[0], t, r(2)) * dt
        this.rotXY[i * 2 + 1] -= evalMinMax(s.rotationOverLifetime3D[1], t, r(2)) * dt
      }
      for (const sub of births) if (rnd() <= sub.probability) this.fire(sub, i, false, dt)
      if (this.trail?.has[i]) this.record(i)
      i++
    }
  }

  /** Adds the particle's position to its trail once it has moved far enough. */
  private record(i: number): void {
    const tr = this.trail!
    const base = i * TRAIL_POINTS
    const n = tr.histCount[i]
    const x = this.pos[i * 3]
    const y = this.pos[i * 3 + 1]
    const z = this.pos[i * 3 + 2]
    if (n) {
      const last = (base + n - 1) * 3
      const min = this.sim.trail!.minVertexDistance
      if (Math.hypot(x - tr.hist[last], y - tr.hist[last + 1], z - tr.hist[last + 2]) < min) return
    }
    if (n === TRAIL_POINTS) {
      tr.hist.copyWithin(base * 3, (base + 1) * 3, (base + TRAIL_POINTS) * 3)
      tr.histTime.copyWithin(base, base + 1, base + TRAIL_POINTS)
    }
    const k = Math.min(n, TRAIL_POINTS - 1)
    tr.hist.set([x, y, z], (base + k) * 3)
    tr.histTime[base + k] = this.age[i]
    tr.histCount[i] = Math.min(TRAIL_POINTS, n + 1)
  }

  private remove(i: number): void {
    const last = --this.count
    if (i === last) return
    this.pos.copyWithin(i * 3, last * 3, last * 3 + 3)
    this.vel.copyWithin(i * 3, last * 3, last * 3 + 3)
    this.age[i] = this.age[last]
    this.life[i] = this.life[last]
    this.size[i] = this.size[last]
    this.rot.copyWithin(i * 2, last * 2, last * 2 + 2)
    this.rotXY.copyWithin(i * 2, last * 2, last * 2 + 2)
    this.sizeYZ.copyWithin(i * 2, last * 2, last * 2 + 2)
    this.rand.copyWithin(i * 4, last * 4, last * 4 + 4)
    this.color.copyWithin(i * 4, last * 4, last * 4 + 4)
    const tr = this.trail
    if (tr) {
      tr.has[i] = tr.has[last]
      tr.histCount[i] = tr.histCount[last]
      tr.hist.copyWithin(i * TRAIL_POINTS * 3, last * TRAIL_POINTS * 3, (last + 1) * TRAIL_POINTS * 3)
      tr.histTime.copyWithin(i * TRAIL_POINTS, last * TRAIL_POINTS, (last + 1) * TRAIL_POINTS)
    }
  }

  /** Columns and rows of the texture sheet: one tile without the module. */
  private tiles(): THREE.Vector2 {
    return new THREE.Vector2(this.sim.sheet?.tilesX ?? 1, this.sim.sheet?.tilesY ?? 1)
  }

  /** Colour of particle `i` now (start colour × colour over lifetime), sRGB. */
  private colorAt(i: number, t: number, out: number[]): number[] {
    const over = this.sim.colorOverLifetime ? evalMinMaxColor(this.sim.colorOverLifetime, t, this.rand[i * 4 + 1]) : undefined
    for (let k = 0; k < 4; k++) out[k] = this.color[i * 4 + k] * (over ? over[k] : 1)
    return out
  }

  private upload(): void {
    const s = this.sim
    const sheet = s.sheet
    const frames = sheet ? (sheet.singleRow ? sheet.tilesX : sheet.tilesX * sheet.tilesY) : 1
    const c = [0, 0, 0, 0]
    const matrix = new THREE.Matrix4()
    const q = new THREE.Quaternion()
    const euler = new THREE.Euler(0, 0, 0, 'YXZ')
    const turned = new THREE.Matrix4()
    const color = new THREE.Color()
    for (let i = 0; i < this.count; i++) {
      const t = this.age[i] / this.life[i]
      const r = (k: number) => this.rand[i * 4 + k]
      this.colorAt(i, t, c)
      const over = s.sizeOverLifetime ? evalMinMax(s.sizeOverLifetime, t, r(2)) : 1
      const size = Math.max(0, this.size[i] * over)
      const [overY, overZ] = s.sizeOverLifetime3D ? s.sizeOverLifetime3D.map((c) => evalMinMax(c, t, r(2))) : [over, over]
      const sizeY = Math.max(0, this.sizeYZ[i * 2] * overY)
      const sizeZ = Math.max(0, this.sizeYZ[i * 2 + 1] * overZ)
      const angle = this.rot[i * 2] + this.rot[i * 2 + 1]
      let frame = 0
      if (sheet) {
        const f = (evalMinMax(sheet.frame, t, r(3)) * sheet.cycles) % 1
        frame = Math.min(frames - 1, Math.floor(f * frames + evalMinMax(sheet.startFrame, 0, r(0)))) % frames
        if (sheet.singleRow) frame += sheet.row * sheet.tilesX
      }
      if (this.meshes) {
        // Mirrored back: the mesh comes from an FBX already in three.js handedness.
        // Z, then X, then Y, like Unity; World alignment undoes the system's rotation first.
        q.setFromEuler(euler.set(this.rotXY[i * 2], this.rotXY[i * 2 + 1], angle))
        if (this.align) {
          turned.compose(new THREE.Vector3(), q, new THREE.Vector3(-size, sizeY, sizeZ))
          matrix.makeTranslation(this.pos[i * 3], this.pos[i * 3 + 1], this.pos[i * 3 + 2]).multiply(this.align).multiply(turned)
        } else matrix.compose(scratch.set(this.pos[i * 3], this.pos[i * 3 + 1], this.pos[i * 3 + 2]), q, new THREE.Vector3(-size, sizeY, sizeZ))
        this.meshes.setMatrixAt(i, matrix)
        if (this.look.blend === 'multiply') {
          // Multiply has no alpha: fading goes towards white, which leaves the background as it is.
          this.meshes.setColorAt(i, color.setRGB(1 - (1 - c[0]) * c[3], 1 - (1 - c[1]) * c[3], 1 - (1 - c[2]) * c[3], THREE.SRGBColorSpace))
        } else {
          this.meshes.setColorAt(i, color.setRGB(c[0], c[1], c[2], this.look.gamma && this.look.custom ? THREE.LinearSRGBColorSpace : THREE.SRGBColorSpace))
          ;(this.meshAlpha!.array as Float32Array)[i] = c[3]
        }
        ;(this.meshFrame!.array as Float32Array)[i] = frame
        if (this.meshStreams) this.writeStreams(this.meshStreams, i, t, size, angle, frame)
        continue
      }
      const qd = this.quads!
      ;(qd.pos.array as Float32Array).set(this.pos.subarray(i * 3, i * 3 + 3), i * 3)
      ;(qd.vel.array as Float32Array).set(this.vel.subarray(i * 3, i * 3 + 3), i * 3)
      const col = qd.color.array as Float32Array
      // Colours are authored in sRGB; the shader works in linear light.
      for (let k = 0; k < 4; k++) col[i * 4 + k] = k < 3 && !this.look.gamma ? toLinear(c[k]) : c[k]
      const srf = qd.srf.array as Float32Array
      srf[i * 3] = size
      srf[i * 3 + 1] = angle
      srf[i * 3 + 2] = frame
      ;(qd.rotXY.array as Float32Array).set(this.rotXY.subarray(i * 2, i * 2 + 2), i * 2)
      ;(qd.sizeY.array as Float32Array)[i] = sizeY
      if (qd.streams) this.writeStreams(qd.streams, i, t, size, angle, frame)
    }
    if (this.meshes) {
      this.meshes.count = this.count
      this.meshes.instanceMatrix.needsUpdate = true
      if (this.meshes.instanceColor) this.meshes.instanceColor.needsUpdate = true
      if (this.meshAlpha) this.meshAlpha.needsUpdate = true
      if (this.meshFrame) this.meshFrame.needsUpdate = true
      if (this.meshStreams) this.meshStreams.s0.needsUpdate = this.meshStreams.s1.needsUpdate = true
    } else if (this.quads) {
      this.quads.geometry.instanceCount = this.count
      for (const a of [this.quads.pos, this.quads.vel, this.quads.color, this.quads.srf, this.quads.rotXY, this.quads.sizeY, this.quads.streams?.s0, this.quads.streams?.s1]) if (a) a.needsUpdate = true
    }
    if (this.trail) this.uploadTrails(c)
  }

  /**
   * Per-particle TEXCOORD0 and TEXCOORD1 for a ported shader, laid out as the renderer's vertex streams say;
   * `uUvMap` marks the components that are the vertex's own UV.
   */
  private streamAttributes(n: number): StreamAttributes {
    const layout = streamTexcoords(this.sim.streams ?? DEFAULT_STREAMS).slice(0, 8)
    const attr = () => new THREE.InstancedBufferAttribute(new Float32Array(n * 4), 4).setUsage(THREE.DynamicDrawUsage)
    const s0 = attr()
    const s1 = attr()
    // A TEXCOORD component no stream fills reads as the GPU's default, (0, 0, 0, 1).
    for (let i = 0; i < n; i++) {
      if (layout.length <= 3) (s0.array as Float32Array)[i * 4 + 3] = 1
      if (layout.length <= 7) (s1.array as Float32Array)[i * 4 + 3] = 1
    }
    const uvMap = [0, 0, 0, 0, 0, 0, 0, 0].map((_, j) => (layout[j] && layout[j].stream >= 4 && layout[j].stream <= 7 ? layout[j].component + 1 : 0))
    return { s0, s1, layout, uniforms: { uUvMap0: { value: new THREE.Vector4(...uvMap.slice(0, 4)) }, uUvMap1: { value: new THREE.Vector4(...uvMap.slice(4)) } } }
  }

  /** Fills particle `i`'s streams. */
  private writeStreams(st: StreamAttributes, i: number, t: number, size: number, angle: number, frame: number): void {
    const s0 = st.s0.array as Float32Array
    const s1 = st.s1.array as Float32Array
    st.layout.forEach(({ stream, component }, j) => {
      const v = this.streamValue(i, stream, component, t, size, angle, frame)
      if (j < 4) s0[i * 4 + j] = v
      else s1[i * 4 + j - 4] = v
    })
  }

  /** One component of a vertex stream for particle `i` (ParticleSystemVertexStream numbers); UVs are filled in by the shader. */
  private streamValue(i: number, stream: number, component: number, t: number, size: number, angle: number, frame: number): number {
    const r = (k: number) => this.rand[i * 4 + (k % 4)]
    if (stream >= 31 && stream <= 38) {
      const curves = stream <= 34 ? this.sim.custom1 : this.sim.custom2
      const curve = curves?.[component]
      // Custom data a curve does not set is 0, as in Unity.
      return curve ? evalMinMax(curve, t, r(component)) : 0
    }
    if (stream >= 23 && stream <= 30) return r(component)
    switch (stream) {
      case 9:
        return frame
      case 10:
        return this.pos[i * 3 + component]
      case 12:
      case 13:
      case 14:
        return size
      case 15:
        return angle
      case 19:
        return this.vel[i * 3 + component]
      case 20:
        return Math.hypot(this.vel[i * 3], this.vel[i * 3 + 1], this.vel[i * 3 + 2])
      case 21:
        return t
      case 22:
        return 1 / this.life[i]
      default:
        return 0
    }
  }

  private uploadTrails(c: number[]): void {
    const tr = this.trail!
    const ts = this.sim.trail!
    const { a, b, width, colorA, colorB, u } = tr.attrs
    let seg = 0
    const max = a.count
    for (let i = 0; i < this.count && seg < max; i++) {
      if (!tr.has[i]) continue
      const base = i * TRAIL_POINTS
      // The particle itself is the newest point.
      const pts = Math.min(tr.histCount[i], TRAIL_POINTS - 1)
      const span = Math.max(1e-3, evalMinMax(ts.lifetime, 0, this.rand[i * 4]) * this.life[i])
      const t = this.age[i] / this.life[i]
      const life = evalMinMaxColor(ts.colorOverLifetime, t, this.rand[i * 4 + 2])
      const own = ts.inheritParticleColor ? this.colorAt(i, t, c) : [1, 1, 1, 1]
      const sizeK = ts.sizeAffectsWidth ? this.size[i] : 1
      const point = (k: number, out: THREE.Vector3) => (k === pts ? out.set(this.pos[i * 3], this.pos[i * 3 + 1], this.pos[i * 3 + 2]) : out.fromArray(tr.hist, (base + k) * 3))
      const pointU = (k: number) => (k === pts ? 0 : Math.min(1, (this.age[i] - tr.histTime[base + k]) / span))
      const pa = new THREE.Vector3()
      const pb = new THREE.Vector3()
      for (let k = 0; k < pts && seg < max; k++) {
        const ua = pointU(k)
        const ub = pointU(k + 1)
        // Points older than the trail's lifetime are gone.
        if (ua >= 1 && ub >= 1) continue
        point(k, pa)
        point(k + 1, pb)
        ;(a.array as Float32Array).set([pa.x, pa.y, pa.z], seg * 3)
        ;(b.array as Float32Array).set([pb.x, pb.y, pb.z], seg * 3)
        ;(width.array as Float32Array).set([evalMinMax(ts.width, ua, 0.5) * sizeK, evalMinMax(ts.width, ub, 0.5) * sizeK], seg * 2)
        for (const [attr, uu] of [
          [colorA, ua],
          [colorB, ub]
        ] as const) {
          const over = evalMinMaxColor(ts.colorOverTrail, uu, 0.5)
          const arr = attr.array as Float32Array
          for (let ch = 0; ch < 4; ch++) {
            const v = over[ch] * life[ch] * own[ch] * (ch === 3 ? 1 - uu : 1)
            arr[seg * 4 + ch] = ch < 3 && !this.look.gamma ? toLinear(v) : v
          }
        }
        ;(u.array as Float32Array).set([ua, ub], seg * 2)
        seg++
      }
    }
    tr.geometry.instanceCount = seg
    for (const attr of Object.values(tr.attrs)) attr.needsUpdate = true
  }

  dispose(): void {
    this.mesh.removeFromParent()
    this.quads?.geometry.dispose()
    this.quads?.material.dispose()
    if (this.meshes) {
      this.meshes.dispose()
      this.meshes.geometry.dispose()
      ;(this.meshes.material as THREE.Material).dispose()
    }
    this.trail?.geometry.dispose()
    this.trail?.material.dispose()
  }
}

/** Shots of an effect playing at once at most (`EffectPlayer.fire`): spamming the button stays smooth. */
const MAX_SHOTS = 12

/** Share of particles at each end of an axis left out of the camera's frame: the odd stray should not shrink the effect. */
const STRAYS = 0.02

/** Every emitter of a prefab, played together like Unity plays the root system. */
export class EffectPlayer {
  paused = false
  speed = 1
  /** Replays a one-shot effect once it is over. */
  loop = true
  /** Seconds since this play started. */
  time = 0
  /** Seed of this play's random numbers, for going back to an earlier moment of it. */
  private seed = newSeed()
  /** Held on the timeline since it last played: moving forward goes on from where it is. */
  private scrubbing = false
  /** Largest side of the textures its emitters use. */
  maxTextureSize = 0
  private wait = 0
  /** Extra plays fired on top (`fire`), each removed once it is over. */
  private shots: EmitterPlayer[][] = []

  constructor(readonly emitters: EmitterPlayer[]) {}

  update(dt: number): void {
    if (this.paused) return
    const step = dt * this.speed
    this.scrubbing = false
    for (const e of this.emitters) e.update(step)
    this.time += step
    if (this.loop && this.emitters.every((e) => e.finished)) {
      // A short pause between plays, so the start of the effect reads.
      this.wait += step
      if (this.wait > 0.5) this.restart()
    }
    for (const shot of this.shots) for (const e of shot) e.update(step)
    const over = this.shots.filter((shot) => shot.every((e) => e.finished))
    if (!over.length) return
    for (const shot of over) for (const e of shot) e.dispose()
    this.shots = this.shots.filter((shot) => !over.includes(shot))
  }

  /** Clears the shots fired, and plays the effect from its start. */
  restart(): void {
    this.seed = newSeed()
    this.replay()
  }

  /** From the start of this same play: the same random numbers, so the same particles. */
  private replay(): void {
    rngState = this.seed
    this.wait = 0
    this.time = 0
    this.clearShots()
    for (const e of this.emitters) e.restart()
  }

  /**
   * Shows the effect `seconds` into this play, worked out in steps of a frame
   * from its start (same particles every time); while held there, moving
   * forward goes on from where it is.
   */
  seek(seconds: number): void {
    const to = Math.max(0, seconds)
    if (!this.scrubbing || to < this.time - 1e-6) this.replay()
    this.scrubbing = true
    this.advance(to - this.time)
  }

  /** How long the timeline runs: a one-shot to its fade-out, a loop one cycle and its particles' lives. Capped at 10 s. */
  timeline(): number {
    const visible = this.emitters.filter((e) => e.emitter.renderMode !== 'none' && !e.isSub)
    if (!visible.length) return 1
    return Math.min(10, Math.max(0.5, ...visible.map((e) => e.emitter.duration + e.emitter.lifetime)))
  }

  /**
   * Plays the effect once more on top of what is playing, like instantiating
   * the prefab again in a game: each emitter gets a copy at the same place
   * (sub emitters linked among the copies), removed when it is over. The
   * oldest shot goes when too many are playing.
   */
  fire(): void {
    if (this.shots.length >= MAX_SHOTS) for (const e of this.shots.shift()!) e.dispose()
    const copies = this.emitters.map((e) => {
      const copy = e.shot()
      copy.isSub = e.isSub
      e.mesh.parent?.add(copy.mesh)
      return copy
    })
    copies.forEach((copy, i) => {
      for (const sub of this.emitters[i].subs) copy.subs.push({ ...sub, player: copies[this.emitters.indexOf(sub.player)] })
      // Sub emitters start empty: they wait for their parents' particles.
      if (copy.isSub) copy.restart()
    })
    this.shots.push(copies)
  }

  private clearShots(): void {
    for (const shot of this.shots) for (const e of shot) e.dispose()
    this.shots = []
  }

  /**
   * The stretch a moving thumbnail covers: a one-shot from start to fade-out,
   * a loop one cycle long once it has filled in (`warm`). Capped so a card stays short.
   */
  span(): { warm: number; length: number } {
    const visible = this.emitters.filter((e) => e.emitter.renderMode !== 'none' && !e.isSub)
    if (!visible.length) return { warm: 0, length: 1 }
    const loops = visible.filter((e) => e.emitter.looping)
    const lifetime = Math.max(...visible.map((e) => e.emitter.lifetime))
    if (loops.length) return { warm: Math.min(3, lifetime), length: Math.min(3, Math.max(1, ...loops.map((e) => e.emitter.duration))) }
    return { warm: 0, length: Math.min(4, Math.max(0.5, ...visible.map((e) => e.emitter.duration + e.emitter.lifetime))) }
  }

  /** Particles alive now, over every emitter. */
  get alive(): number {
    return this.emitters.reduce((n, e) => n + e.alive, 0)
  }

  /** Runs the effect forward without drawing (thumbnails). */
  advance(seconds: number): void {
    // Steps of a 30th of a second, then what is left: exactly `seconds`, so frames at set times land on them.
    for (let left = seconds; left > 1e-6; left -= FRAME) {
      const step = Math.min(FRAME, left)
      for (const e of this.emitters) e.update(step)
      this.time += step
    }
  }

  /**
   * Where the particles really go: plays the effect through once without
   * drawing and keeps, per emitter, the box nearly all of its particles stay
   * in (a few strays left out), padded by their size. Per emitter, so a few
   * big ones (a glow, a ground ring) are framed next to a hundred sparks.
   * Starts the effect over afterwards. Falls back on `extent()` when nothing
   * was emitted.
   */
  measuredExtent(): THREE.Box3 {
    const { warm, length } = this.span()
    const samples = this.emitters.map(() => ({ axes: [[], [], []] as [number[], number[], number[]], sizes: [] as number[] }))
    this.restart()
    const step = 1 / 30
    for (let t = 0, frame = 0; t < warm + length; t += step, frame++) {
      for (const e of this.emitters) e.update(step)
      if (frame % 3) continue
      this.emitters.forEach((e, i) =>
        e.sample((at, size) => {
          samples[i].axes[0].push(at.x)
          samples[i].axes[1].push(at.y)
          samples[i].axes[2].push(at.z)
          samples[i].sizes.push(size)
        })
      )
    }
    this.restart()
    if (samples.reduce((n, s) => n + s.sizes.length, 0) < 8) return this.extent()
    const pick = (values: number[], q: number) => values.sort((a, b) => a - b)[Math.min(values.length - 1, Math.floor(q * values.length))]
    const box = new THREE.Box3()
    for (const { axes, sizes } of samples) {
      if (!sizes.length) continue
      const pad = pick(sizes, 0.5) / 2
      const lo = axes.map((v) => pick(v, STRAYS) - pad)
      const hi = axes.map((v) => pick(v, 1 - STRAYS) + pad)
      box.union(new THREE.Box3(new THREE.Vector3(lo[0], lo[1], lo[2]), new THREE.Vector3(hi[0], hi[1], hi[2])))
    }
    return box
  }

  /** How far particles may fly, around each emitter's object (for framing the camera). */
  extent(): THREE.Box3 {
    const box = new THREE.Box3()
    for (const e of this.emitters) {
      if (!e.mesh.parent || e.emitter.renderMode === 'none' || e.isSub) continue
      e.mesh.parent.updateWorldMatrix(true, false)
      const center = e.mesh.parent.getWorldPosition(new THREE.Vector3())
      const scale = e.mesh.parent.getWorldScale(new THREE.Vector3())
      const r = e.reach() * Math.max(Math.abs(scale.x), Math.abs(scale.y), Math.abs(scale.z), 1e-3)
      box.union(new THREE.Box3().setFromCenterAndSize(center, new THREE.Vector3(r, r, r).multiplyScalar(2)))
    }
    return box
  }

  dispose(): void {
    this.clearShots()
    for (const e of this.emitters) e.dispose()
  }
}
