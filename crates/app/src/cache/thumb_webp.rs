//! PNG thumbnails re-encoded as lossy WebP.
//!
//! WKWebView ignores `toDataURL('image/webp')` and answers PNG, six to seven times larger than the
//! WebP that 1.x pages saved. The cache, the shared index pushes and the bundle of a team all carry
//! those files, so a PNG is re-encoded before it is stored.

use std::io::Cursor;

use png::{ColorType, Decoder, Transformations};

/// Lossy quality the 1.x pages asked for (`toDataURL('image/webp', 0.85)`).
pub const WEBP_QUALITY: f32 = 85.0;

/// Largest picture re-encoded, in pixels. A thumbnail is a few hundred pixels wide; this stops a
/// small PNG that declares a huge picture from taking all the memory.
const MAX_PIXELS: u64 = 4096 * 4096;

/// The WebP image of `png`, or `None` when `png` is not a PNG this can read, is too large, or
/// the WebP would not be smaller (the PNG is then kept as it is).
pub fn png_to_webp(png: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = Decoder::new(Cursor::new(png));
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let (width, height) = reader.info().size();
    if u64::from(width) * u64::from(height) > MAX_PIXELS || width == 0 || height == 0 {
        return None;
    }
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    pixels.truncate(frame.buffer_size());

    let rgba = to_rgba(&pixels, frame.color_type)?;
    // Without transparency the WebP carries no alpha plane, which is smaller.
    let (pixels4, _) = rgba.as_chunks::<4>();
    let encoded = if pixels4.iter().all(|p| p[3] == 255) {
        let rgb: Vec<u8> = pixels4.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        webp::Encoder::from_rgb(&rgb, width, height).encode(WEBP_QUALITY)
    } else {
        webp::Encoder::from_rgba(&rgba, width, height).encode(WEBP_QUALITY)
    };
    (encoded.len() < png.len()).then(|| encoded.to_vec())
}

/// 8-bit RGBA pixels from what the decoder returns once palettes are expanded and 16-bit is stripped.
fn to_rgba(pixels: &[u8], color: ColorType) -> Option<Vec<u8>> {
    match color {
        ColorType::Rgba => Some(pixels.to_vec()),
        ColorType::Rgb => Some(pixels.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect()),
        ColorType::GrayscaleAlpha => Some(pixels.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect()),
        ColorType::Grayscale => Some(pixels.iter().flat_map(|&g| [g, g, g, 255]).collect()),
        ColorType::Indexed => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD_A: &[u8] = include_bytes!("../testdata/thumb-card-a.png");
    const CARD_B: &[u8] = include_bytes!("../testdata/thumb-card-b.png");

    /// Sizes of the WebP files that a 1.x page saved for the same two cards.
    const CARD_A_1X_WEBP_BYTES: usize = 4_238;
    const CARD_B_1X_WEBP_BYTES: usize = 5_184;

    fn encode_png(width: u32, height: u32, color: ColorType, pixels: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(pixels).unwrap();
        out
    }

    #[test]
    fn re_encodes_a_card_as_webp_about_the_size_of_the_1x_file() {
        for (png, one_x) in [(CARD_A, CARD_A_1X_WEBP_BYTES), (CARD_B, CARD_B_1X_WEBP_BYTES)] {
            let webp = png_to_webp(png).unwrap();
            assert_eq!(&webp[0..4], b"RIFF");
            assert_eq!(&webp[8..12], b"WEBP");
            assert!(webp.len() * 2 <= one_x * 3, "{} bytes against {one_x} from 1.x", webp.len());
            let decoded = webp::Decoder::new(&webp).decode().unwrap();
            assert_eq!((decoded.width(), decoded.height()), (320, 240));
        }
    }

    #[test]
    fn keeps_transparency() {
        let pixels: Vec<u8> = (0..64 * 64).flat_map(|i| [200, 40, 40, if i % 64 < 32 { 0 } else { 255 }]).collect();
        let webp = png_to_webp(&encode_png(64, 64, ColorType::Rgba, &pixels)).unwrap();
        let decoded = webp::Decoder::new(&webp).decode().unwrap();
        assert!(decoded.is_alpha());
        assert_eq!(decoded[3], 0);
        assert_eq!(decoded[(63 * 4) + 3], 255);
    }

    #[test]
    fn reads_grayscale_and_opaque_rgb() {
        let gray: Vec<u8> = (0..64 * 64).map(|i| (i % 251) as u8).collect();
        let rgb: Vec<u8> = (0..64 * 64).flat_map(|i| [(i % 251) as u8, 90, 200]).collect();
        // A tiny picture may not shrink; one that does is a WebP of the same size.
        for png in [encode_png(64, 64, ColorType::Grayscale, &gray), encode_png(64, 64, ColorType::Rgb, &rgb)] {
            if let Some(webp) = png_to_webp(&png) {
                let decoded = webp::Decoder::new(&webp).decode().unwrap();
                assert_eq!((decoded.width(), decoded.height()), (64, 64));
            }
        }
    }

    #[test]
    fn refuses_what_is_not_a_readable_png_or_is_too_large() {
        assert!(png_to_webp(b"").is_none());
        assert!(png_to_webp(b"RIFF....WEBPVP8 ").is_none());
        assert!(png_to_webp(&CARD_A[..CARD_A.len() / 2]).is_none());
        let huge = encode_png(4097, 4096, ColorType::Grayscale, &vec![7; 4097 * 4096]);
        assert!(png_to_webp(&huge).is_none());
    }
}
