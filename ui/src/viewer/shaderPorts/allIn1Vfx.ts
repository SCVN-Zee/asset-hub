/**
 * AllIn1Vfx (the "All In 1 Vfx Toolkit" asset): its particle shader ported to
 * WebGL line by line, so effects made with it look as they do in Unity. The
 * shader turns its features on per material with keywords; each becomes a
 * #define here. Mesh particles bring their normal and view direction (MESH_PARTICLE),
 * which rim and backface tint read; billboards face the camera. Left out: what needs the scene (soft particles, depth glow,
 * screen distortion, fog, light and shadow), the camera distance fade and
 * vertex offset. Vertex streams come in as Unity lays them out: `vTex0` is
 * TEXCOORD0 (uv, then the seed and the alpha stream), `vTex1` TEXCOORD1.
 */

export const ALLIN1_SHADER = /^AllIn1Vfx\/AllIn1Vfx(BuiltIn|GrabPass)?$/

const FLOATS = [
  '_Alpha', '_Glow', '_GlowGlobal', '_ShapeXSpeed', '_ShapeYSpeed', '_ShapeContrast', '_ShapeBrightness', '_ShapeDistortAmount', '_ShapeDistortXSpeed', '_ShapeDistortYSpeed',
  '_ShapeRotationOffset', '_ShapeRotationSpeed', '_ShapeColorWeight', '_ShapeAlphaWeight',
  '_Shape2XSpeed', '_Shape2YSpeed', '_Shape2Contrast', '_Shape2Brightness', '_Shape2DistortAmount', '_Shape2DistortXSpeed', '_Shape2DistortYSpeed',
  '_Shape2RotationOffset', '_Shape2RotationSpeed', '_Shape2ColorWeight', '_Shape2AlphaWeight',
  '_Shape3XSpeed', '_Shape3YSpeed', '_Shape3Contrast', '_Shape3Brightness', '_Shape3DistortAmount', '_Shape3DistortXSpeed', '_Shape3DistortYSpeed',
  '_Shape3RotationOffset', '_Shape3RotationSpeed', '_Shape3ColorWeight', '_Shape3AlphaWeight', '_Shape1MaskPow',
  '_PixelateSize', '_TwistUvAmount', '_TwistUvPosX', '_TwistUvPosY', '_TwistUvRadius', '_HandDrawnAmount', '_HandDrawnSpeed', '_ShakeUvSpeed', '_ShakeUvX', '_ShakeUvY',
  '_WaveAmount', '_WaveSpeed', '_WaveStrength', '_WaveX', '_WaveY', '_RoundWaveStrength', '_RoundWaveSpeed',
  '_DistortAmount', '_DistortTexXSpeed', '_DistortTexYSpeed', '_TextureScrollXSpeed', '_TextureScrollYSpeed', '_TrailWidthPower',
  '_FadeAmount', '_FadeTransition', '_FadePower', '_FadeScrollXSpeed', '_FadeScrollYSpeed', '_FadeBurnWidth', '_FadeBurnGlow',
  '_AlphaFadeAmount', '_AlphaFadeSmooth', '_AlphaFadePow', '_ColorGradingMidPoint', '_ColorRampLuminosity', '_ColorRampBlend', '_PosterizeNumColors',
  '_RimBias', '_RimScale', '_RimPower', '_RimIntensity', '_RimAddAmount', '_RimErodesAlpha', '_HsvShift', '_HsvSaturation', '_HsvBright',
  '_MaskPow', '_AlphaStepMin', '_AlphaStepMax', '_AlphaCutoffValue', '_TimingSeed',
  '_OffsetSh1', '_OffsetSh2', '_OffsetSh3', '_Sh1BlendOffset', '_Sh2BlendOffset', '_Sh3BlendOffset', '_DistortionBlend'
]
const COLORS = ['_Color', '_GlowColor', '_ShapeColor', '_Shape2Color', '_Shape3Color', '_FadeBurnColor', '_ColorGradingLight', '_ColorGradingMiddle', '_ColorGradingDark', '_RimColor', '_BackFaceTint', '_FrontFaceTint']
/** Texture slots; each has its tiling and offset as `<slot>_ST`. */
export const ALLIN1_TEXTURES = [
  '_MainTex', '_ShapeDistortTex', '_Shape2Tex', '_Shape2DistortTex', '_Shape3Tex', '_Shape3DistortTex', '_Shape1MaskTex', '_DistortTex', '_TrailWidthGradient',
  '_FadeTex', '_FadeBurnTex', '_ColorRampTex', '_ColorRampTexGradient', '_GlowTex', '_MaskTex'
]

/** Keywords this port reproduces; others are left off. */
const KEYWORDS = new Set([
  'PIXELATE_ON', 'TWISTUV_ON', 'DOODLE_ON', 'SHAKEUV_ON', 'WAVEUV_ON', 'ROUNDWAVEUV_ON', 'POLARUV_ON', 'POLARUVDISTORT_ON', 'DISTORT_ON', 'TEXTURESCROLL_ON', 'TRAILWIDTH_ON',
  'SHAPE1CONTRAST_ON', 'SHAPE1DISTORT_ON', 'SHAPE1ROTATE_ON', 'SHAPE1SHAPECOLOR_ON',
  'SHAPE2_ON', 'SHAPE2CONTRAST_ON', 'SHAPE2DISTORT_ON', 'SHAPE2ROTATE_ON', 'SHAPE2SHAPECOLOR_ON',
  'SHAPE3_ON', 'SHAPE3CONTRAST_ON', 'SHAPE3DISTORT_ON', 'SHAPE3ROTATE_ON', 'SHAPE3SHAPECOLOR_ON',
  'SHAPEADD_ON', 'SPLITRGBA_ON', 'SHAPE1MASK_ON', 'PREMULTIPLYCOLOR_ON', 'PREMULTIPLYALPHA_ON',
  'FADE_ON', 'FADEBURN_ON', 'ALPHAFADE_ON', 'ALPHAFADEUSESHAPE1_ON', 'ALPHAFADEUSEREDCHANNEL_ON', 'ALPHAFADETRANSPARENCYTOO_ON', 'ALPHAFADEINPUTSTREAM_ON',
  'BACKFACETINT_ON', 'COLORGRADING_ON', 'COLORRAMP_ON', 'COLORRAMPGRAD_ON', 'POSTERIZE_ON', 'RIM_ON', 'GLOW_ON', 'GLOWTEX_ON', 'HSV_ON', 'MASK_ON', 'ALPHASMOOTHSTEP_ON', 'ALPHACUTOFF_ON', 'ADDITIVECONFIG_ON',
  'OFFSETSTREAM_ON', 'SHAPEWEIGHTS_ON', 'SCREENDISTORTION_ON'
])

/** The fragment shader for one material's keywords. Expects `vTex0`, `vTex1` and `vColor` from the particle vertex shader. */
export function allIn1Fragment(keywords: readonly string[]): string {
  const defines = keywords.filter((k) => KEYWORDS.has(k)).map((k) => `#define ${k} 1`)
  return [
    ...defines,
    ...FLOATS.map((f) => `uniform float ${f};`),
    ...COLORS.map((c) => `uniform vec4 ${c};`),
    ...ALLIN1_TEXTURES.flatMap((t) => [`uniform sampler2D ${t};`, `uniform vec4 ${t}_ST;`]),
    'uniform vec4 _MainTex_TexelSize;',
    'uniform float uTime;',
    // Unity's TRANSFORM_TEX macro pastes tokens (name##_ST), which WebGL's preprocessor cannot: written out instead.
    BODY.replace(/TRANSFORM_TEX\(([\w.]+), (\w+)\)/g, '(($1) * $2_ST.xy + $2_ST.zw)')
  ].join('\n')
}

/** Unity's BlendMode numbers (`Blend [_SrcMode] [_DstMode]`) as three.js blend factors. */
export const UNITY_BLEND_FACTORS = [200, 201, 208, 202, 209, 204, 203, 206, 207, 210, 205] as const

// Ported from AllIn1Vfx.shader (frag) and AllIn1VfxFunctions.cginc. HLSL's `x % 1` is mod(x, 1.0) here;
// `_Time` used as a scalar is its x component, t/20, as HLSL truncates the vector.
const BODY = /* glsl */ `
varying vec4 vTex0;
varying vec4 vTex1;
varying vec4 vColor;
#ifdef MESH_PARTICLE
varying vec3 vNormalW;
varying vec3 vViewDirW;
#endif

vec4 sampleScroll(sampler2D tex, vec2 uv, float xs, float ys, float time) {
  uv.x += mod(time * xs, 1.0);
  uv.y += mod(time * ys, 1.0);
  return texture2D(tex, uv);
}
vec2 rotateUvs(vec2 uv, float rotation, vec4 st) {
  vec2 center = vec2(0.5 * st.x + st.z, 0.5 * st.y + st.w);
  float c = cos(rotation);
  float s = sin(rotation);
  uv -= center;
  uv = vec2(c * uv.x - s * uv.y, s * uv.x + c * uv.y);
  return uv + center;
}
float remap01ToSigned(float v) { return -1.0 + v * 2.0; }
float easeOutQuint(float x) { return 1.0 - pow(1.0 - x, 5.0); }

void main() {
  float seed = vTex0.z + _TimingSeed;
  float time = uTime + seed;
  float time20 = uTime / 20.0 + seed;
  vec2 uv = vTex0.xy;
  float fadeAmount = _FadeAmount;
  float fadeTransition = _FadeTransition;
  float alphaFadeAmount = _AlphaFadeAmount;
  float alphaFadeSmooth = _AlphaFadeSmooth;
  vec4 vcol = vColor;
  #ifdef MESH_PARTICLE
  vec3 normalW = normalize(vNormalW);
  vec3 viewDirW = normalize(vViewDirW);
  #else
  // Billboards face the camera: the normal is the view direction.
  vec3 normalW = vec3(0.0, 0.0, 1.0);
  vec3 viewDirW = normalW;
  #endif

  #ifdef PIXELATE_ON
  uv = floor(uv * _PixelateSize) / _PixelateSize;
  #endif
  #ifdef TWISTUV_ON
  {
    vec2 tempUv = uv - vec2(_TwistUvPosX * _MainTex_ST.x, _TwistUvPosY * _MainTex_ST.y);
    float radius = _TwistUvRadius * (_MainTex_ST.x + _MainTex_ST.y) / 2.0;
    float percent = (radius - length(tempUv)) / radius;
    float theta = percent * percent * (2.0 * sin(_TwistUvAmount)) * 8.0;
    float s = sin(theta);
    float c = cos(theta);
    float beta = max(sign(radius - length(tempUv)), 0.0);
    tempUv = vec2(dot(tempUv, vec2(c, -s)), dot(tempUv, vec2(s, c))) * beta + tempUv * (1.0 - beta);
    uv = tempUv + vec2(_TwistUvPosX * _MainTex_ST.x, _TwistUvPosY * _MainTex_ST.y);
  }
  #endif
  #ifdef DOODLE_ON
  {
    float speed = (floor(time20 * 20.0 * _HandDrawnSpeed) / _HandDrawnSpeed) * _HandDrawnSpeed;
    vec2 uvCopy = vec2(sin((uv.x * _HandDrawnAmount + speed) * 4.0), cos((uv.y * _HandDrawnAmount + speed) * 4.0));
    uv = mix(uv, uv + uvCopy, 0.0005 * _HandDrawnAmount);
  }
  #endif
  #ifdef SHAKEUV_ON
  uv += vec2(sin(time20 * _ShakeUvSpeed * 50.0) * _ShakeUvX * 0.012, cos(time20 * _ShakeUvSpeed * 50.0) * _ShakeUvY * 0.01);
  #endif
  #ifdef WAVEUV_ON
  {
    vec2 uvWave = vec2(_WaveX * _MainTex_ST.x, _WaveY * _MainTex_ST.y) - uv;
    float angWave = sqrt(dot(uvWave, uvWave)) * _WaveAmount - mod(time * _WaveSpeed, 360.0);
    uv = uv + normalize(uvWave) * sin(angWave) * (_WaveStrength / 1000.0);
  }
  #endif
  #ifdef ROUNDWAVEUV_ON
  {
    float xWave = 0.5 * _MainTex_ST.x - uv.x;
    float yWave = (0.5 * _MainTex_ST.y - uv.y) * (_MainTex_TexelSize.w / _MainTex_TexelSize.z);
    float ripple = -sqrt(xWave * xWave + yWave * yWave);
    uv += mod(sin((ripple + time * (_RoundWaveSpeed / 10.0)) / 0.015) * (_RoundWaveStrength / 10.0), 1.0);
  }
  #endif
  #ifdef POLARUV_ON
  vec2 prePolarUvs = uv;
  uv -= vec2(0.5);
  uv = vec2(atan(uv.y, uv.x) / 6.28318530718, length(uv) * 2.0);
  uv *= _MainTex_ST.xy;
  #endif
  #ifdef DISTORT_ON
  {
    vec2 distortUvs = TRANSFORM_TEX(uv, _DistortTex);
    #ifndef POLARUVDISTORT_ON
    distortUvs = TRANSFORM_TEX(vTex0.xy, _DistortTex);
    #endif
    distortUvs.x += mod(time20 * _DistortTexXSpeed, 1.0);
    distortUvs.y += mod(time20 * _DistortTexYSpeed, 1.0);
    float d = (texture2D(_DistortTex, distortUvs).r - 0.5) * 0.2 * _DistortAmount;
    uv += vec2(d);
  }
  #endif
  #ifdef TEXTURESCROLL_ON
  uv.x += mod(time * _TextureScrollXSpeed, 1.0);
  uv.y += mod(time * _TextureScrollYSpeed, 1.0);
  #endif
  #ifdef TRAILWIDTH_ON
  {
    float width = pow(texture2D(_TrailWidthGradient, uv).r, _TrailWidthPower);
    uv.y = (uv.y * 2.0 - 1.0) / width * 0.5 + 0.5;
    if (uv.y < 0.0 || uv.y > 1.0) discard;
  }
  #endif

  vec2 shape1Uv = TRANSFORM_TEX(uv, _MainTex);
  #ifdef OFFSETSTREAM_ON
  shape1Uv += vTex1.xy * _OffsetSh1;
  #endif
  #ifdef SHAPE1DISTORT_ON
  {
    vec2 du = TRANSFORM_TEX(vTex0.xy, _ShapeDistortTex);
    #ifdef POLARUVDISTORT_ON
    du = TRANSFORM_TEX(uv, _ShapeDistortTex);
    #endif
    du.x += mod((time + seed) * _ShapeDistortXSpeed, 1.0);
    du.y += mod((time + seed) * _ShapeDistortYSpeed, 1.0);
    shape1Uv += vec2((texture2D(_ShapeDistortTex, du).r - 0.5) * 0.2 * _ShapeDistortAmount);
  }
  #endif
  #ifdef SHAPE1ROTATE_ON
  shape1Uv = rotateUvs(shape1Uv, _ShapeRotationOffset + mod(_ShapeRotationSpeed * time, 6.28318530718), _MainTex_ST);
  #endif
  vec4 shape1 = sampleScroll(_MainTex, shape1Uv, _ShapeXSpeed, _ShapeYSpeed, time);
  #ifdef SHAPE1SHAPECOLOR_ON
  shape1.a = shape1.r;
  shape1.rgb = _ShapeColor.rgb;
  #else
  shape1 *= _ShapeColor;
  #endif
  #ifdef SHAPE1CONTRAST_ON
  #ifdef SHAPE1SHAPECOLOR_ON
  shape1.a = clamp((shape1.a - 0.5) * _ShapeContrast + 0.5 + _ShapeBrightness, 0.0, 1.0);
  #else
  shape1.rgb = max(vec3(0.0), (shape1.rgb - vec3(0.5)) * _ShapeContrast + vec3(0.5) + _ShapeBrightness);
  #endif
  #endif

  #ifdef SHAPE2_ON
  vec2 shape2Uv = TRANSFORM_TEX(uv, _Shape2Tex);
  #ifdef OFFSETSTREAM_ON
  shape2Uv += vTex1.xy * _OffsetSh2;
  #endif
  #ifdef SHAPE2DISTORT_ON
  {
    vec2 du = TRANSFORM_TEX(vTex0.xy, _Shape2DistortTex);
    #ifdef POLARUVDISTORT_ON
    du = TRANSFORM_TEX(uv, _Shape2DistortTex);
    #endif
    du.x += mod((time + seed) * _Shape2DistortXSpeed, 1.0);
    du.y += mod((time + seed) * _Shape2DistortYSpeed, 1.0);
    shape2Uv += vec2((texture2D(_Shape2DistortTex, du).r - 0.5) * 0.2 * _Shape2DistortAmount);
  }
  #endif
  #ifdef SHAPE2ROTATE_ON
  shape2Uv = rotateUvs(shape2Uv, _Shape2RotationOffset + mod(_Shape2RotationSpeed * time, 6.28318530718), _Shape2Tex_ST);
  #endif
  vec4 shape2 = sampleScroll(_Shape2Tex, shape2Uv, _Shape2XSpeed, _Shape2YSpeed, time);
  #ifdef SHAPE2SHAPECOLOR_ON
  shape2.a = shape2.r;
  shape2.rgb = _Shape2Color.rgb;
  #else
  shape2 *= _Shape2Color;
  #endif
  #ifdef SHAPE2CONTRAST_ON
  #ifdef SHAPE2SHAPECOLOR_ON
  shape2.a = max(0.0, (shape2.a - 0.5) * _Shape2Contrast + 0.5 + _Shape2Brightness);
  #else
  shape2.rgb = max(vec3(0.0), (shape2.rgb - vec3(0.5)) * _Shape2Contrast + vec3(0.5) + _Shape2Brightness);
  #endif
  #endif
  #endif

  #ifdef SHAPE3_ON
  vec2 shape3Uv = TRANSFORM_TEX(uv, _Shape3Tex);
  #ifdef OFFSETSTREAM_ON
  shape3Uv += vTex1.xy * _OffsetSh3;
  #endif
  #ifdef SHAPE3DISTORT_ON
  {
    vec2 du = TRANSFORM_TEX(vTex0.xy, _Shape3DistortTex);
    #ifdef POLARUVDISTORT_ON
    du = TRANSFORM_TEX(uv, _Shape3DistortTex);
    #endif
    du.x += mod((time + seed) * _Shape3DistortXSpeed, 1.0);
    du.y += mod((time + seed) * _Shape3DistortYSpeed, 1.0);
    shape3Uv += vec2((texture2D(_Shape3DistortTex, du).r - 0.5) * 0.3 * _Shape3DistortAmount);
  }
  #endif
  #ifdef SHAPE3ROTATE_ON
  shape3Uv = rotateUvs(shape3Uv, _Shape3RotationOffset + mod(_Shape3RotationSpeed * time, 6.28318530718), _Shape3Tex_ST);
  #endif
  vec4 shape3 = sampleScroll(_Shape3Tex, shape3Uv, _Shape3XSpeed, _Shape3YSpeed, time);
  #ifdef SHAPE3SHAPECOLOR_ON
  shape3.a = shape3.r;
  shape3.rgb = _Shape3Color.rgb;
  #else
  shape3 *= _Shape3Color;
  #endif
  #ifdef SHAPE3CONTRAST_ON
  #ifdef SHAPE3SHAPECOLOR_ON
  shape3.a = max(0.0, (shape3.a - 0.5) * _Shape3Contrast + 0.5 + _Shape3Brightness);
  #else
  shape3.rgb = max(vec3(0.0), (shape3.rgb - vec3(0.5)) * _Shape3Contrast + vec3(0.5) + _Shape3Brightness);
  #endif
  #endif
  #endif

  vec4 col = shape1;
  #ifdef SHAPE2_ON
  float w1c = _ShapeColorWeight, w1a = _ShapeAlphaWeight, w2c = _Shape2ColorWeight, w2a = _Shape2AlphaWeight;
  #ifdef SHAPEWEIGHTS_ON
  w1c = max(0.0, w1c + vTex1.z * _Sh1BlendOffset);
  w1a = max(0.0, w1a + vTex1.z * _Sh1BlendOffset);
  w2c = max(0.0, w2c + vTex1.z * _Sh2BlendOffset);
  w2a = max(0.0, w2a + vTex1.z * _Sh2BlendOffset);
  #endif
  #ifndef SPLITRGBA_ON
  w1a = w1c;
  w2a = w2c;
  #endif
  #ifdef SHAPE3_ON
  float w3c = _Shape3ColorWeight, w3a = _Shape3AlphaWeight;
  #ifdef SHAPEWEIGHTS_ON
  w3c = max(0.0, w3c + vTex1.z * _Sh3BlendOffset);
  w3a = max(0.0, w3a + vTex1.z * _Sh3BlendOffset);
  #endif
  #ifndef SPLITRGBA_ON
  w3a = w3c;
  #endif
  #ifdef SHAPEADD_ON
  col.rgb = shape1.rgb * w1c + shape2.rgb * w2c + shape3.rgb * w3c;
  col.a = clamp(max(shape3.a * w3a, max(shape1.a * w1a, shape2.a * w2a)), 0.0, 1.0);
  #else
  col.rgb = (shape1.rgb * w1c) * (shape2.rgb * w2c) * (shape3.rgb * w3c);
  col.a = clamp((shape1.a * w1a) * (shape2.a * w2a) * (shape3.a * w3a), 0.0, 1.0);
  #endif
  #else
  #ifdef SHAPEADD_ON
  col.rgb = shape1.rgb * w1c + shape2.rgb * w2c;
  col.a = clamp(max(shape1.a * w1a, shape2.a * w2a), 0.0, 1.0);
  #else
  col.rgb = (shape1.rgb * w1c) * (shape2.rgb * w2c);
  col.a = clamp((shape1.a * w1a) * (shape2.a * w2a), 0.0, 1.0);
  #endif
  #endif
  #endif

  #ifdef SHAPE1MASK_ON
  col = mix(col, shape1, pow(texture2D(_Shape1MaskTex, TRANSFORM_TEX(uv, _Shape1MaskTex)).r, _Shape1MaskPow));
  #endif

  float luminance = 0.0;
  #ifdef PREMULTIPLYCOLOR_ON
  luminance = 0.3 * col.r + 0.59 * col.g + 0.11 * col.b;
  col.a = min(luminance, col.a);
  #endif

  col.rgb *= _Color.rgb * vcol.rgb;
  #ifdef PREMULTIPLYALPHA_ON
  col.rgb *= col.a;
  #endif

  #ifndef PREMULTIPLYCOLOR_ON
  luminance = 0.3 * col.r + 0.59 * col.g + 0.11 * col.b;
  #endif

  #if (defined(FADE_ON) || defined(ALPHAFADE_ON)) && defined(ALPHAFADEINPUTSTREAM_ON)
  // The fade comes from a vertex stream (TEXCOORD0.w); the colour's alpha becomes plain transparency.
  col.a *= vcol.a;
  vcol.a = vTex0.w;
  #endif

  #ifdef FADE_ON
  {
    float preFadeAlpha = col.a;
    fadeAmount = clamp(fadeAmount + (1.0 - vcol.a), 0.0, 1.0);
    fadeTransition = max(0.01, fadeTransition * easeOutQuint(clamp(fadeAmount, 0.0, 1.0)));
    vec2 fadeUv = uv + seed;
    fadeUv.x += mod(time * _FadeScrollXSpeed, 1.0);
    fadeUv.y += mod(time * _FadeScrollYSpeed, 1.0);
    #if defined(ADDITIVECONFIG_ON) && !defined(PREMULTIPLYCOLOR_ON)
    preFadeAlpha *= luminance;
    #endif
    fadeAmount = clamp(pow(fadeAmount, _FadePower), 0.0, 1.0);
    float fadeSample = texture2D(_FadeTex, TRANSFORM_TEX(fadeUv, _FadeTex)).r;
    float edge = clamp(smoothstep(0.0, fadeTransition, remap01ToSigned(1.0 - fadeAmount) + fadeSample), 0.0, 1.0);
    col.a *= edge;
    #ifdef FADEBURN_ON
    float burn = clamp(smoothstep(0.0, fadeTransition + _FadeBurnWidth, remap01ToSigned(1.0 - fadeAmount) + fadeSample), 0.0, 1.0);
    col.rgb += (edge - burn) * texture2D(_FadeBurnTex, TRANSFORM_TEX(fadeUv, _FadeBurnTex)).rgb * _FadeBurnColor.rgb * _FadeBurnGlow * preFadeAlpha;
    #endif
    #ifdef ALPHAFADETRANSPARENCYTOO_ON
    col.a *= 1.0 - fadeAmount;
    #endif
  }
  #endif

  #ifdef ALPHAFADE_ON
  {
    alphaFadeAmount = clamp(alphaFadeAmount + (1.0 - vcol.a), 0.0, 1.0);
    alphaFadeAmount = clamp(pow(alphaFadeAmount, _AlphaFadePow), 0.0, 1.0);
    alphaFadeSmooth = max(0.01, alphaFadeSmooth * easeOutQuint(clamp(alphaFadeAmount, 0.0, 1.0)));
    #ifdef ALPHAFADEUSESHAPE1_ON
    float l = shape1.r;
    #else
    float l = luminance;
    #endif
    l = clamp(l - 0.001, 0.0, 1.0);
    #ifdef ALPHAFADEUSEREDCHANNEL_ON
    col.a *= col.r;
    #endif
    col.a = clamp(col.a, 0.0, 1.0);
    col.a *= clamp(smoothstep(0.0, alphaFadeSmooth, remap01ToSigned(1.0 - alphaFadeAmount) + l), 0.0, 1.0);
    #ifdef ALPHAFADETRANSPARENCYTOO_ON
    col.a *= 1.0 - alphaFadeAmount;
    #endif
  }
  #endif

  #ifdef BACKFACETINT_ON
  col.rgb = mix(col.rgb * _BackFaceTint.rgb, col.rgb * _FrontFaceTint.rgb, step(0.0, dot(normalW, viewDirW)));
  #endif

  #ifdef COLORGRADING_ON
  col.rgb *= mix(mix(_ColorGradingDark.rgb, _ColorGradingMiddle.rgb, luminance / _ColorGradingMidPoint),
    mix(_ColorGradingMiddle.rgb, _ColorGradingLight.rgb, (luminance - _ColorGradingMidPoint) / (1.0 - _ColorGradingMidPoint)), step(_ColorGradingMidPoint, luminance));
  #endif

  #ifdef COLORRAMP_ON
  {
    float rampL = clamp(luminance + _ColorRampLuminosity, 0.0, 1.0);
    #ifdef COLORRAMPGRAD_ON
    vec4 ramp = texture2D(_ColorRampTexGradient, vec2(rampL, 0.0));
    #else
    vec4 ramp = texture2D(_ColorRampTex, vec2(rampL, 0.0));
    #endif
    col.rgb = mix(col.rgb, ramp.rgb, _ColorRampBlend);
    col.a = mix(col.a, clamp(col.a * ramp.a, 0.0, 1.0), _ColorRampBlend);
  }
  #endif

  #ifdef POSTERIZE_ON
  col.rgb = floor(col.rgb / (1.0 / _PosterizeNumColors)) * (1.0 / _PosterizeNumColors);
  #endif

  #ifdef RIM_ON
  {
    float NdV = max(1.0 - abs(dot(normalW, viewDirW)), 0.0);
    float rimFactor = clamp(_RimBias + _RimScale * pow(NdV, _RimPower), 0.0, 1.0);
    vec3 rimCol = _RimColor.rgb * rimFactor * _RimIntensity;
    col.rgb = mix(col.rgb * (rimCol + vec3(1.0)), col.rgb + rimCol, _RimAddAmount);
    col.a = clamp(col.a * (1.0 - rimFactor * _RimErodesAlpha), 0.0, 1.0);
  }
  #endif

  #ifdef GLOW_ON
  {
    float glowMask = 1.0;
    #ifdef GLOWTEX_ON
    glowMask = texture2D(_GlowTex, TRANSFORM_TEX(uv, _GlowTex)).r;
    #endif
    col.rgb *= _GlowGlobal * glowMask;
    float glowMult = 1.0;
    #ifdef ADDITIVECONFIG_ON
    glowMult = luminance;
    #endif
    col.rgb += _GlowColor.rgb * _Glow * glowMask * col.a * glowMult;
  }
  #endif

  #ifdef HSV_ON
  {
    float cosH = _HsvBright * _HsvSaturation * cos(_HsvShift * 3.14159265 / 180.0);
    float sinH = _HsvBright * _HsvSaturation * sin(_HsvShift * 3.14159265 / 180.0);
    vec3 c = col.rgb;
    col.rgb = vec3(
      (.299 * _HsvBright + .701 * cosH + .168 * sinH) * c.x + (.587 * _HsvBright - .587 * cosH + .330 * sinH) * c.y + (.114 * _HsvBright - .114 * cosH - .497 * sinH) * c.z,
      (.299 * _HsvBright - .299 * cosH - .328 * sinH) * c.x + (.587 * _HsvBright + .413 * cosH + .035 * sinH) * c.y + (.114 * _HsvBright - .114 * cosH + .292 * sinH) * c.z,
      (.299 * _HsvBright - .3 * cosH + 1.25 * sinH) * c.x + (.587 * _HsvBright - .588 * cosH - 1.05 * sinH) * c.y + (.114 * _HsvBright + .886 * cosH - .203 * sinH) * c.z);
  }
  #endif

  #ifdef MASK_ON
  {
    vec2 maskUv = uv;
    #ifdef POLARUV_ON
    maskUv = prePolarUvs;
    #endif
    vec4 m = texture2D(_MaskTex, TRANSFORM_TEX(maskUv, _MaskTex));
    col.a *= pow(min(m.r, m.a), _MaskPow);
  }
  #endif

  #ifdef ALPHASMOOTHSTEP_ON
  col.a = smoothstep(_AlphaStepMin, _AlphaStepMax, col.a);
  #endif
  #ifdef ALPHACUTOFF_ON
  if ((1.0 - _AlphaCutoffValue) - (1.0 - col.a) - 0.01 < 0.0) discard;
  #endif

  #if !defined(FADE_ON) && !defined(ALPHAFADE_ON)
  col.a *= _Alpha * vcol.a;
  #else
  col.a *= _Alpha;
  #endif
  #ifdef SCREENDISTORTION_ON
  // The GrabPass variant blends its colour into the screen behind it, bent (_DistortionBlend of the way):
  // with no screen to sample, that part is left see-through rather than drawn as a flat card.
  float keep = 1.0 - clamp(_DistortionBlend, 0.0, 1.0);
  col *= keep;
  #endif
  #ifdef ADDITIVECONFIG_ON
  col.rgb *= col.a;
  #endif

  if (col.a < 0.003 && col.r + col.g + col.b < 0.003) discard;
  gl_FragColor = col;
  // A Gamma project blends the values as they are; a Linear one is shown through sRGB.
  #ifndef GAMMA_SPACE
  #include <colorspace_fragment>
  #endif
}
`
