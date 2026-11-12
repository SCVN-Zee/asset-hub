/**
 * Shared parts of the particle shaders: GLSL ES 3.00 in the style of Unity's compiler output (`hlslcc_mtx4x4...`
 * uniforms, `in_POSITION0` attributes), which is the form Luna's particle instancing patch rewrites. Registration is
 * in `../boot.ts`. Each shader has its own module in this folder and is listed in `./index.ts`.
 */
import type { LunaShaderState } from '../../protocol'

/** `UnityEngine.Rendering.BlendMode` values the shaders use. */
export const Blend = {
  Zero: 0,
  One: 1,
  DstColor: 2,
  SrcColor: 3,
  SrcAlpha: 5,
  OneMinusSrcColor: 6,
  OneMinusSrcAlpha: 10
} as const

/** `UnityEngine.Rendering.ColorWriteMask`: red, green and blue. */
export const COLOR_MASK_RGB = 14
export const COLOR_MASK_ALL = 15

/** Render queues. */
export const Queue = { Geometry: 2000, AlphaTest: 2450, Transparent: 3000 } as const

export interface ParticleShaderDef {
  /** Name that `Shader.Find` and materials use. */
  name: string
  vertex: string
  fragment: string
  /** `BlendMode` of the source and the destination. */
  blend: { src: number; dst: number }
  zWrite: boolean
  /** `CullMode`: 0 off, 1 front, 2 back. */
  cull: 0 | 1 | 2
  colorMask: number
  queue: number
}

export interface ShaderOptions {
  keywords: readonly string[]
  /** The material's own state, for modules that take it from there. */
  state?: LunaShaderState
}

export interface ParticleShaderModule {
  /** Id that materials name in `LunaMaterialSpec.shader`. */
  id: string
  /** Unity shaders this module draws, for humans and for the mapping tests. */
  unityNames: readonly string[]
  /** The shader for the options of one material. Modules with fixed state and code ignore the options. */
  build(options: ShaderOptions): ParticleShaderDef
}

/** The state of a transparent particle shader that blends with `src` and `dst`, no depth write, no culling. */
export function transparentState(src: number, dst: number): Pick<ParticleShaderDef, 'blend' | 'zWrite' | 'cull' | 'colorMask' | 'queue'> {
  return { blend: { src, dst }, zWrite: false, cull: 0, colorMask: COLOR_MASK_RGB, queue: Queue.Transparent }
}

/**
 * The vertex shader of a particle: world transform, vertex colour, one texture coordinate. `st` names the uniform
 * that scales and offsets the coordinate; null passes the raw coordinate on (shaders that apply their own tiling).
 */
export function particleVertex(st: string | null): string {
  return `#version 300 es

uniform 	vec4 hlslcc_mtx4x4unity_ObjectToWorld[4];
uniform 	vec4 hlslcc_mtx4x4unity_MatrixVP[4];
${st ? `uniform 	vec4 ${st};\n` : ''}in highp vec4 in_POSITION0;
in mediump vec4 in_COLOR0;
in highp vec2 in_TEXCOORD0;
out mediump vec4 vs_COLOR0;
out highp vec2 vs_TEXCOORD0;
vec4 u_xlat0;
vec4 u_xlat1;
void main()
{
    u_xlat0 = in_POSITION0.yyyy * hlslcc_mtx4x4unity_ObjectToWorld[1];
    u_xlat0 = hlslcc_mtx4x4unity_ObjectToWorld[0] * in_POSITION0.xxxx + u_xlat0;
    u_xlat0 = hlslcc_mtx4x4unity_ObjectToWorld[2] * in_POSITION0.zzzz + u_xlat0;
    u_xlat0 = u_xlat0 + hlslcc_mtx4x4unity_ObjectToWorld[3];
    u_xlat1 = u_xlat0.yyyy * hlslcc_mtx4x4unity_MatrixVP[1];
    u_xlat1 = hlslcc_mtx4x4unity_MatrixVP[0] * u_xlat0.xxxx + u_xlat1;
    u_xlat1 = hlslcc_mtx4x4unity_MatrixVP[2] * u_xlat0.zzzz + u_xlat1;
    gl_Position = hlslcc_mtx4x4unity_MatrixVP[3] * u_xlat0.wwww + u_xlat1;
    vs_COLOR0 = in_COLOR0;
    vs_TEXCOORD0.xy = ${st ? `in_TEXCOORD0.xy * ${st}.xy + ${st}.zw` : 'in_TEXCOORD0.xy'};
    return;
}
`
}

/**
 * A fragment shader with one texture read, written as Unity's compiler writes it. `body` runs with `texel` (the
 * texture sample) and `vertexColor` in scope and assigns `SV_Target0`. `uniforms` are extra uniform lines.
 */
export function particleFragment(uniforms: string, body: string, sampler = '_MainTex'): string {
  return `#version 300 es

precision highp float;
precision highp int;
${uniforms}
uniform mediump sampler2D ${sampler};
in mediump vec4 vs_COLOR0;
in highp vec2 vs_TEXCOORD0;
layout(location = 0) out mediump vec4 SV_Target0;
void main()
{
    mediump vec4 texel = texture(${sampler}, vs_TEXCOORD0.xy);
    mediump vec4 vertexColor = vs_COLOR0;
${body}
    return;
}
`
}

/**
 * Turns a fragment shader written for three.js (GLSL ES 1.00 with `varying vTex0/vTex1/vColor`, `texture2D`,
 * `gl_FragColor`, `uTime`) into the Luna form, so the existing ports are reused as they are. Luna's particle
 * instancing offers position, colour and one texture coordinate. The streams the ports read beyond that (the
 * Custom1 data in TEXCOORD1, the per-particle seed) are zero here.
 */
export function esToLunaFragment(es1: string): string {
  const source = es1
    .replace(/^varying .*$/gm, '')
    .replace(/^uniform float uTime;$/gm, '')
    .replace(/^\s*#include <colorspace_fragment>\s*$/gm, '')
    .replace(/\btexture2D\(/g, 'texture(')
    .replace(/\bgl_FragColor\b/g, 'SV_Target0')
  return `#version 300 es

precision highp float;
precision highp int;
precision highp sampler2D;
uniform vec4 _Time;
#define uTime (_Time.y)
in mediump vec4 vs_COLOR0;
in highp vec2 vs_TEXCOORD0;
#define vColor vs_COLOR0
#define vTex0 vec4(vs_TEXCOORD0, 0.0, 0.0)
#define vTex1 vec4(0.0)
layout(location = 0) out mediump vec4 SV_Target0;
${source}`
}
