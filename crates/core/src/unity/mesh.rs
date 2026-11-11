//! Meshes Unity saves as YAML (`.mesh`, or a Mesh inside an `.asset`): effect packs ship their waves,
//! rings and slashes this way. Read from the vertex data and index buffer stored as hex; compressed
//! meshes and data in a separate stream file are not. Port of `src/core/unity/mesh.ts`.

use serde::{Deserialize, Serialize};

use crate::js::{self, to_number};
use crate::re;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnityMesh {
    /// x, y, z per vertex, in Unity's axes.
    pub positions: Vec<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normals: Option<Vec<f32>>,
    /// First UV set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uvs: Option<Vec<f32>>,
    /// Triangles, from every sub-mesh.
    pub indices: Vec<u32>,
}

/// VertexFormat (Unity 2019+): bytes per component.
const FORMAT_SIZES: [usize; 12] = [4, 2, 1, 1, 2, 2, 1, 1, 2, 2, 4, 4];

fn half(h: u16) -> f64 {
    let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 0x1f) as i32;
    let f = (h & 0x3ff) as f64;
    if e == 0 {
        return s * 2f64.powi(-14) * (f / 1024.0);
    }
    if e == 31 {
        return if f != 0.0 { f64::NAN } else { s * f64::INFINITY };
    }
    s * 2f64.powi(e - 15) * (1.0 + f / 1024.0)
}

/// Reads one component of the given VertexFormat at a byte offset.
fn read_component(format: usize, d: &[u8], at: usize) -> f64 {
    let u16_at = || u16::from_le_bytes([d[at], d[at + 1]]);
    let u32_at = || u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]);
    match format {
        0 => f32::from_bits(u32_at()) as f64,
        1 => half(u16_at()),
        2 => d[at] as f64 / 255.0,
        3 => (d[at] as i8 as f64 / 127.0).max(-1.0),
        4 => u16_at() as f64 / 65535.0,
        5 => (u16_at() as i16 as f64 / 32767.0).max(-1.0),
        6 => d[at] as f64,
        7 => d[at] as i8 as f64,
        8 => u16_at() as f64,
        9 => u16_at() as i16 as f64,
        10 => u32_at() as f64,
        _ => u32_at() as i32 as f64,
    }
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes().chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap_or("0"), 16).unwrap_or(0)).collect()
}

/// `text.slice(from, to)` where a missing index is JavaScript's -1 (the end, less one character).
fn slice_minus_one(text: &str, from: Option<usize>, to: Option<usize>) -> &str {
    let last = (0..text.len()).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
    let (f, t) = (from.unwrap_or(last), to.unwrap_or(last));
    if f >= t {
        ""
    } else {
        &text[f..t]
    }
}

struct Channel {
    stream: usize,
    offset: usize,
    format: usize,
    dim: usize,
}

/// The first Mesh in a YAML asset; `None` when it holds none or keeps its data elsewhere.
pub fn parse_unity_mesh(asset_text: &str) -> Option<UnityMesh> {
    let text = asset_text.replace("\r\n", "\n");
    let start = re!(r"^Mesh:\s*$", "m").search(&text)?;
    let num = |key: &str, from: usize| js::group1(&format!(r"^\s*{key}: (-?\d+)"), "m", js::slice_from(&text, from)).map_or(f64::NAN, to_number);
    if num("m_MeshCompression", start) > 0.0 {
        return None;
    }
    let vertex_at = text[start..].find("m_VertexData:").map(|i| i + start)?;
    let count_f = num("m_VertexCount", vertex_at);
    let data = hex_bytes(re!(r"_typelessdata: ([0-9a-f]*)").exec(&text[vertex_at..]).and_then(|c| c.get(1)).unwrap_or(""));
    if !(count_f > 0.0) || data.is_empty() {
        return None;
    }
    let count = count_f as usize;

    // Channels in order: position, normal, tangent, color, uv0..uv7, blend weights, blend indices.
    let channels_text = slice_minus_one(&text, text[vertex_at..].find("m_Channels:").map(|i| i + vertex_at), text[vertex_at..].find("m_DataSize:").map(|i| i + vertex_at));
    let channels: Vec<Channel> = re!(r"stream: (\d+)\s*\n\s*offset: (\d+)\s*\n\s*format: (\d+)\s*\n\s*dimension: (\d+)", "g")
        .find_all(channels_text)
        .iter()
        .map(|m| {
            let n = |i: usize| m.get(i).unwrap_or("0").parse::<usize>().unwrap_or(usize::MAX);
            Channel { stream: n(1), offset: n(2), format: n(3), dim: n(4) & 0xf }
        })
        .collect();
    // Each stream is interleaved; streams follow one another, each starting on a 16-byte boundary.
    if channels.iter().any(|c| c.dim != 0 && c.stream > 64) {
        return None;
    }
    let size_of = |format: usize| FORMAT_SIZES.get(format).copied().unwrap_or(4);
    let mut strides: Vec<usize> = Vec::new();
    for c in channels.iter().filter(|c| c.dim != 0) {
        if strides.len() <= c.stream {
            strides.resize(c.stream + 1, 0);
        }
        strides[c.stream] = strides[c.stream].max(c.offset + c.dim * size_of(c.format));
    }
    let mut stream_start = Vec::new();
    let mut at = 0usize;
    for stride in &strides {
        stream_start.push(at);
        at += (stride * count).div_ceil(16) * 16;
    }
    let read = |ch: usize, comps: usize| -> Option<Vec<f32>> {
        let c = channels.get(ch)?;
        if c.dim == 0 || c.format >= FORMAT_SIZES.len() {
            return None;
        }
        let size = FORMAT_SIZES[c.format];
        let mut out = vec![0f32; count * comps];
        for i in 0..count {
            for k in 0..comps.min(c.dim) {
                let byte = stream_start[c.stream] + i * strides[c.stream] + c.offset + k * size;
                if byte + size > data.len() {
                    return None;
                }
                out[i * comps + k] = read_component(c.format, &data, byte) as f32;
            }
        }
        Some(out)
    };
    let positions = read(0, 3)?;

    let wide = num("m_IndexFormat", start) == 1.0;
    let ib = hex_bytes(re!(r"m_IndexBuffer: ([0-9a-f]*)").exec(&text[start..]).and_then(|c| c.get(1)).unwrap_or(""));
    let indices: Vec<u32> = if wide {
        ib.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    } else {
        ib.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as u32).collect()
    };
    if indices.is_empty() || indices.iter().any(|&i| i as usize >= count) {
        return None;
    }
    Some(UnityMesh { positions, normals: read(1, 3), uvs: read(4, 2), indices })
}
