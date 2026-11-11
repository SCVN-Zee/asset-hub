//! The flattened (composite) image every PSD stores after its layers (`src/core/image/psd.ts`).
//! Unity projects often use `.psd` textures, and sometimes PSD files carry a `.png` name.

use crate::image::{buffer, DecodedImage, ImageError};
use crate::js::View;

pub fn is_psd(bytes: &[u8]) -> bool {
    bytes.len() > 26 && bytes.starts_with(b"8BPS")
}

/// PackBits, as used for PSD scanlines. Reads past the end of `src` give zero, like an out-of-range
/// typed-array read.
fn unpack_bits(src: &[u8], start: usize, length: usize, out: &mut [u8], out_start: usize, out_length: usize) {
    let byte = |i: usize| src.get(i).copied().unwrap_or(0);
    let mut i = start;
    let mut o = out_start;
    let end = start + length;
    let out_end = out_start + out_length;
    while i < end && o < out_end {
        let n = byte(i) as i8;
        i += 1;
        if n >= 0 {
            let count = (n as usize + 1).min(out_end - o).min(end - i);
            let from = i.min(src.len());
            let to = (i + count).min(src.len());
            out[o..o + (to - from)].copy_from_slice(&src[from..to]);
            i += n as usize + 1;
            o += count;
        } else if n != -128 {
            let count = (1 - n as i32) as usize;
            let count = count.min(out_end - o);
            let value = byte(i);
            i += 1;
            out[o..o + count].fill(value);
            o += count;
        }
    }
}

/// Composite image of an 8- or 16-bit RGB or grayscale PSD (not PSB), raw or RLE compressed.
/// `Ok(None)` for anything else (CMYK, 32-bit, ZIP).
pub fn decode_psd(bytes: &[u8]) -> Result<Option<DecodedImage>, ImageError> {
    if !is_psd(bytes) {
        return Ok(None);
    }
    let dv = View::new(bytes);
    if dv.u16(4)? != 1 {
        return Ok(None); // version 2 is PSB
    }
    let channels = dv.u16(12)? as usize;
    let height = dv.u32(14)? as usize;
    let width = dv.u32(18)? as usize;
    let depth = dv.u16(22)?;
    let mode = dv.u16(24)?; // 1 grayscale, 3 RGB
    if (depth != 8 && depth != 16) || (mode != 1 && mode != 3) || width == 0 || height == 0 {
        return Ok(None);
    }

    let mut pos = 26usize;
    for _ in 0..3 {
        // Color mode data, image resources, layer and mask info: skip each by its length.
        if pos + 4 > bytes.len() {
            return Ok(None);
        }
        pos += 4 + dv.u32(pos)? as usize;
    }
    if pos + 2 > bytes.len() {
        return Ok(None);
    }
    let compression = dv.u16(pos)?;
    pos += 2;

    let bpc = depth as usize / 8;
    let row_bytes = width as u64 * bpc as u64;
    let plane_bytes = row_bytes * height as u64;
    let planes = channels.min(if mode == 3 { 4 } else { 2 });
    let mut raw = buffer(plane_bytes * planes as u64)?;
    let (row_bytes, plane_bytes) = (row_bytes as usize, plane_bytes as usize);
    if compression == 0 {
        let from = pos.min(bytes.len());
        let to = (pos + raw.len()).min(bytes.len());
        raw[..to - from].copy_from_slice(&bytes[from..to]);
    } else if compression == 1 {
        // Byte counts for every scanline of every channel come first.
        let counts = pos;
        let mut data = pos + channels * height * 2;
        for c in 0..channels {
            for y in 0..height {
                let len = dv.u16(counts + (c * height + y) * 2)? as usize;
                if c < planes {
                    unpack_bits(bytes, data, len, &mut raw, c * plane_bytes + y * row_bytes, row_bytes);
                }
                data += len;
            }
        }
    } else {
        return Ok(None);
    }

    let mut out = buffer(width as u64 * height as u64 * 4)?;
    // 16-bit: the high byte. A plane the file does not have reads as 0.
    let sample = |plane: usize, i: usize| raw.get(plane * plane_bytes + i * bpc).copied().unwrap_or(0);
    let rgb = mode == 3;
    for i in 0..width * height {
        let r = sample(0, i);
        out[i * 4] = r;
        out[i * 4 + 1] = if rgb { sample(1, i) } else { r };
        out[i * 4 + 2] = if rgb { sample(2, i) } else { r };
        out[i * 4 + 3] = if planes > if rgb { 3 } else { 1 } { sample(if rgb { 3 } else { 1 }, i) } else { 255 };
    }
    Ok(Some(DecodedImage { width: width as u32, height: height as u32, data: out }))
}
