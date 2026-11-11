Shader "AllIn1Vfx/AllIn1Vfx"
{
    Properties
    {
        _SrcMode("SrcMode", float) = 5
        [HDR] _GlowColor("Glow Color", Color) = (1,1,1,1)
        [NoScaleOffset] _MainTex("Shape Texture", 2D) = "white" {}
    }
    SubShader
    {
        Blend [_SrcMode] [_DstMode]
        Pass { CGPROGRAM
            #include "UnityCG.cginc"
            #include "AllIn1Include.cginc"
            ENDCG }
    }
}
