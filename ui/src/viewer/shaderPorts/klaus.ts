/**
 * VFX_Klaus (stylized explosion, hit and slash packs): one formula in four
 * shaders. The texture packs its channels — R brightness, G the dissolve
 * mask, B a self-distortion — and Custom1 (TEXCOORD1) drives them over each
 * particle's life: x dissolve, y its sharpness, z distortion, w soft particles
 * (left out: no depth here). `_add` blends SrcAlpha One with an emission
 * power, `_apb` SrcAlpha OneMinusSrcAlpha.
 */

export const KLAUS_SHADER = /^VFX_Klaus\/(fx_explosion|Fx_Hit&Slash)_(add|apb)$/

export const KLAUS_FRAGMENT = /* glsl */ `
uniform sampler2D _Main_Tex;
uniform vec4 _Main_Tex_ST;
uniform float _Emission_Power;
uniform float uAdditive;
uniform float uTime;
varying vec4 vTex0;
varying vec4 vTex1;
varying vec4 vColor;
void main() {
  vec4 k = vTex1;
  vec2 uv = vTex0.xy * _Main_Tex_ST.xy + _Main_Tex_ST.zw;
  vec4 tex = texture2D(_Main_Tex, uv + texture2D(_Main_Tex, uv).b * k.z);
  float edge = k.x * -1.0 * k.y;
  float dissolve = clamp(edge + (tex.g + (1.0 - k.x)) * (1.0 - edge) / (k.x * 0.1 + 1.0), 0.0, 1.0);
  vec4 col = tex.r * vColor * dissolve * (uAdditive > 0.5 ? _Emission_Power : 1.0);
  if (col.a < 0.003 && col.r + col.g + col.b < 0.003) discard;
  gl_FragColor = col;
  // A Gamma project blends the values as they are; a Linear one is shown through sRGB.
  #ifndef GAMMA_SPACE
  #include <colorspace_fragment>
  #endif
}
`
