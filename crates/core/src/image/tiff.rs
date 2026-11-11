//! The first image of a TIFF file (`src/core/image/tiff.ts`). Unity imports `.tif` textures
//! (Photoshop saves them), browsers cannot decode them at all. Covers what painting tools write:
//! uncompressed, LZW, Deflate/ZIP and PackBits; 8 or 16 bits per sample; grey, grey + alpha, RGB and
//! RGBA; strips or tiles; the horizontal predictor. Anything else (CMYK, floating point, JPEG inside
//! TIFF) is left undecoded.

use std::collections::HashMap;

use crate::image::{buffer, DecodedImage, ImageError};
use crate::js::View;

pub fn is_tiff(bytes: &[u8]) -> bool {
    bytes.len() > 8 && (bytes.starts_with(&[0x49, 0x49, 42, 0]) || bytes.starts_with(&[0x4d, 0x4d, 0, 42]))
}

const WIDTH: u16 = 256;
const HEIGHT: u16 = 257;
const BITS: u16 = 258;
const COMPRESSION: u16 = 259;
const PHOTOMETRIC: u16 = 262;
const STRIP_OFFSETS: u16 = 273;
const SAMPLES: u16 = 277;
const ROWS_PER_STRIP: u16 = 278;
const STRIP_BYTE_COUNTS: u16 = 279;
const PLANAR: u16 = 284;
const PREDICTOR: u16 = 317;
const TILE_WIDTH: u16 = 322;
const TILE_LENGTH: u16 = 323;
const TILE_OFFSETS: u16 = 324;
const TILE_BYTE_COUNTS: u16 = 325;
const SAMPLE_FORMAT: u16 = 339;

/// Decompressed bytes of one strip or tile (zlib data, as `DecompressionStream('deflate')` reads it).
fn inflate(data: &[u8]) -> Result<Vec<u8>, ImageError> {
    miniz_oxide::inflate::decompress_to_vec_zlib(data).map_err(|e| ImageError::Inflate(format!("{:?}", e.status)))
}

/// TIFF's LZW: MSB-first codes, 9 to 12 bits, Clear 256, End 257, widths growing one code early.
fn lzw(data: &[u8], expected: usize) -> Result<Vec<u8>, ImageError> {
    let mut out = buffer(expected as u64)?;
    let mut o = 0usize;
    let mut bit = 0usize;
    let read = |bit: &mut usize, width: usize| {
        let mut v = 0usize;
        for i in 0..width {
            let byte = data.get((*bit + i) >> 3).copied().unwrap_or(0);
            v = (v << 1) | ((byte >> (7 - ((*bit + i) & 7))) & 1) as usize;
        }
        *bit += width;
        v
    };
    let reset = || {
        let mut table: Vec<Vec<u8>> = (0..=255u8).map(|i| vec![i]).collect();
        table.push(Vec::new());
        table.push(Vec::new());
        table
    };
    let mut table = reset();
    let mut width = 9usize;
    let mut prev: Option<Vec<u8>> = None;
    while o < expected && bit + width <= data.len() * 8 {
        let code = read(&mut bit, width);
        if code == 257 {
            break;
        }
        if code == 256 {
            table = reset();
            width = 9;
            prev = None;
            continue;
        }
        let entry: Vec<u8> = if code < table.len() {
            table[code].clone()
        } else if let Some(p) = &prev {
            let mut e = p.clone();
            e.push(p[0]);
            e
        } else {
            break;
        };
        let n = entry.len().min(expected - o);
        out[o..o + n].copy_from_slice(&entry[..n]);
        o += n;
        if let Some(p) = &prev {
            let mut added = p.clone();
            added.push(entry[0]);
            table.push(added);
        }
        prev = Some(entry);
        // "Early change": the width grows when the next code would not fit one code before the limit.
        if table.len() + 1 >= 1 << width && width < 12 {
            width += 1;
        }
    }
    Ok(out)
}

fn pack_bits(data: &[u8], expected: usize) -> Result<Vec<u8>, ImageError> {
    let mut out = buffer(expected as u64)?;
    let mut o = 0usize;
    let mut i = 0usize;
    while i < data.len() && o < expected {
        let n = data[i] as i8;
        i += 1;
        if n >= 0 {
            let count = (n as usize + 1).min(expected - o);
            let from = i.min(data.len());
            let to = (i + count).min(data.len());
            out[o..o + (to - from)].copy_from_slice(&data[from..to]);
            o += count;
            i += n as usize + 1;
        } else if n != -128 {
            let count = ((1 - n as i32) as usize).min(expected - o);
            let value = data.get(i).copied().unwrap_or(0);
            i += 1;
            out[o..o + count].fill(value);
            o += count;
        }
    }
    Ok(out)
}

/// The first image of a TIFF as RGBA, or `Ok(None)` for what is not covered.
pub fn decode_tiff(bytes: &[u8]) -> Result<Option<DecodedImage>, ImageError> {
    if !is_tiff(bytes) {
        return Ok(None);
    }
    let le = bytes[0] == 0x49;
    let view = View::with_endian(bytes, le);
    let ifd = view.u32(4)? as usize;
    if ifd + 2 > bytes.len() {
        return Ok(None);
    }
    // Values are `None` where a byte-typed entry points past the end of the file (`undefined` in JavaScript).
    let mut tags: HashMap<u16, Vec<Option<u32>>> = HashMap::new();
    for i in 0..view.u16(ifd)? as usize {
        let at = ifd + 2 + i * 12;
        let kind = view.u16(at + 2)?;
        let count = view.u32(at + 4)? as u64;
        let size: u64 = match kind {
            3 => 2,
            4 => 4,
            _ => 1,
        };
        // Values that fit in four bytes sit in the entry; the rest are pointed at.
        let base = if count * size <= 4 { at + 8 } else { view.u32(at + 8)? as usize };
        let mut values = Vec::new();
        for k in 0..count.min(1 << 20) as usize {
            values.push(match kind {
                3 => Some(view.u16(base + k * 2)? as u32),
                4 => Some(view.u32(base + k * 4)?),
                _ => bytes.get(base + k).map(|b| *b as u32),
            });
        }
        tags.insert(view.u16(at)?, values);
    }
    let one = |tag: u16, fallback: u32| tags.get(&tag).and_then(|v| v.first().copied().flatten()).unwrap_or(fallback);
    let width = one(WIDTH, 0) as usize;
    let height = one(HEIGHT, 0) as usize;
    let samples = one(SAMPLES, 1) as usize;
    let bits = one(BITS, 1);
    let compression = one(COMPRESSION, 1);
    let photometric = one(PHOTOMETRIC, 2);
    let predictor = one(PREDICTOR, 1);
    if width == 0 || height == 0 || (bits != 8 && bits != 16) || one(PLANAR, 1) != 1 || one(SAMPLE_FORMAT, 1) != 1 {
        return Ok(None);
    }
    if photometric > 2 || !(1..=4).contains(&samples) || (photometric == 2 && samples < 3) {
        return Ok(None);
    }
    if ![1, 5, 8, 32946, 32773].contains(&compression) {
        return Ok(None);
    }

    let bytes_per_sample = bits as usize / 8;
    let tiled = tags.contains_key(&TILE_OFFSETS);
    let block_w = if tiled { one(TILE_WIDTH, width as u32) as usize } else { width };
    let block_h = if tiled { one(TILE_LENGTH, height as u32) as usize } else { (one(ROWS_PER_STRIP, height as u32) as usize).min(height) };
    let empty: Vec<Option<u32>> = Vec::new();
    let offsets = tags.get(&if tiled { TILE_OFFSETS } else { STRIP_OFFSETS }).unwrap_or(&empty);
    let counts = tags.get(&if tiled { TILE_BYTE_COUNTS } else { STRIP_BYTE_COUNTS }).unwrap_or(&empty);
    // A zero block width would divide by zero below, where JavaScript gets Infinity and NaN.
    let across = if block_w == 0 { 0 } else { width.div_ceil(block_w) };
    let mut rgba = buffer(width as u64 * height as u64 * 4)?;
    let row_bytes = block_w as u64 * samples as u64 * bytes_per_sample as u64;
    let expected = row_bytes * block_h as u64;
    let row_bytes = row_bytes as usize;

    for (b, offset) in offsets.iter().enumerate() {
        let raw: &[u8] = match *offset {
            Some(off) => {
                let start = (off as usize).min(bytes.len());
                let end = (off as usize + counts.get(b).copied().flatten().unwrap_or(0) as usize).min(bytes.len());
                if end > start {
                    &bytes[start..end]
                } else {
                    &[]
                }
            }
            None => &[],
        };
        let mut block: Vec<u8> = match compression {
            5 => lzw(raw, expected as usize)?,
            8 | 32946 => inflate(raw)?,
            32773 => pack_bits(raw, expected as usize)?,
            _ => raw.to_vec(),
        };
        // Horizontal predictor: each sample is stored as the difference to the one before it in its row.
        if predictor == 2 {
            for y in 0..block_h {
                let row = y * row_bytes;
                if row >= block.len() {
                    break;
                }
                if bytes_per_sample == 1 {
                    let mut x = samples;
                    while x < block_w * samples && row + x < block.len() {
                        block[row + x] = block[row + x].wrapping_add(block[row + x - samples]);
                        x += 1;
                    }
                } else {
                    let at = |block: &[u8], i: usize| -> u16 {
                        let (a, b) = (block.get(row + i * 2).copied().unwrap_or(0) as u16, block.get(row + i * 2 + 1).copied().unwrap_or(0) as u16);
                        if le {
                            a | (b << 8)
                        } else {
                            (a << 8) | b
                        }
                    };
                    let mut x = samples;
                    while x < block_w * samples && row + x * 2 + 1 < block.len() {
                        let v = at(&block, x).wrapping_add(at(&block, x - samples));
                        let (hi, lo) = ((v >> 8) as u8, (v & 255) as u8);
                        block[row + x * 2] = if le { lo } else { hi };
                        block[row + x * 2 + 1] = if le { hi } else { lo };
                        x += 1;
                    }
                }
            }
        }
        let x0 = if tiled && across > 0 { (b % across) * block_w } else { 0 };
        let y0 = if tiled && across > 0 { (b / across) * block_h } else { b * block_h };
        let mut y = 0;
        while y < block_h && y0 + y < height {
            let mut x = 0;
            while x < block_w && x0 + x < width {
                let s = (y * block_w + x) * samples;
                // 16-bit samples keep their high byte.
                let v = |k: usize| -> u8 {
                    let at = if bytes_per_sample == 1 { s + k } else { (s + k) * 2 + usize::from(le) };
                    block.get(at).copied().unwrap_or(0)
                };
                let d = ((y0 + y) * width + x0 + x) * 4;
                if photometric == 2 {
                    rgba[d] = v(0);
                    rgba[d + 1] = v(1);
                    rgba[d + 2] = v(2);
                    rgba[d + 3] = if samples > 3 { v(3) } else { 255 };
                } else {
                    let g = if photometric == 0 { 255 - v(0) } else { v(0) };
                    rgba[d] = g;
                    rgba[d + 1] = g;
                    rgba[d + 2] = g;
                    rgba[d + 3] = if samples > 1 { v(1) } else { 255 };
                }
                x += 1;
            }
            y += 1;
        }
    }
    Ok(Some(DecodedImage { width: width as u32, height: height as u32, data: rgba }))
}
