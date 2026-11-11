//! Format, sample rate and channels from an audio file's header (`src/core/audio/header.ts`). Web Audio
//! resamples what it decodes, so the source rate has to be read from the file.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioHeader {
    /// `WAV`, `OGG`, `MP3`, `AIFF`, or the upper-cased file extension.
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<u32>,
}

fn header(format: &str, sample_rate: Option<u32>, channels: Option<u32>) -> AudioHeader {
    AudioHeader { format: format.to_string(), sample_rate, channels }
}

fn mp3_rate(version: u8, index: u8) -> Option<u32> {
    let table: &[u32] = match version {
        0 => &[11025, 12000, 8000],
        2 => &[22050, 24000, 16000],
        3 => &[44100, 48000, 32000],
        _ => &[],
    };
    table.get(index as usize).copied()
}

fn u16_le(b: &[u8], at: usize) -> u32 {
    u16::from_le_bytes([b[at], b[at + 1]]) as u32
}

fn u32_le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub fn audio_header(name: &str, bytes: &[u8]) -> AudioHeader {
    let n = bytes.len();
    if n >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        // Chunks after the RIFF header: find "fmt ".
        let mut i = 12usize;
        while i + 16 <= n {
            let size = u32_le(bytes, i + 4) as usize;
            if &bytes[i..i + 4] == b"fmt " {
                return header("WAV", Some(u32_le(bytes, i + 12)), Some(u16_le(bytes, i + 10)));
            }
            i += 8 + size + (size % 2);
        }
        return header("WAV", None, None);
    }
    if n >= 4 && &bytes[0..4] == b"OggS" {
        let needle = b"\x01vorbis";
        let limit = n.min(512);
        let at = (0..).take_while(|i| i + needle.len() <= limit).find(|&i| &bytes[i..i + needle.len()] == needle);
        if let Some(at) = at.filter(|at| at + 16 <= n) {
            return header("OGG", Some(u32_le(bytes, at + 12)), Some(bytes[at + 11] as u32));
        }
        return header("OGG", None, None);
    }
    if n >= 12 && &bytes[0..4] == b"FORM" && (&bytes[8..12] == b"AIFF" || &bytes[8..12] == b"AIFC") {
        return header("AIFF", None, None);
    }
    // MP3: skip an ID3v2 tag, then read the first frame header.
    let mut i = 0usize;
    if n >= 10 && &bytes[0..3] == b"ID3" {
        i = 10 + (((bytes[6] & 0x7f) as usize) << 21) + (((bytes[7] & 0x7f) as usize) << 14) + (((bytes[8] & 0x7f) as usize) << 7) + (bytes[9] & 0x7f) as usize;
    }
    let end = n.saturating_sub(4).min(i + 4096);
    while i < end {
        if bytes[i] == 0xff && (bytes[i + 1] & 0xe0) == 0xe0 {
            let version = (bytes[i + 1] >> 3) & 3;
            if let Some(rate) = mp3_rate(version, (bytes[i + 2] >> 2) & 3) {
                return header("MP3", Some(rate), Some(if (bytes[i + 3] >> 6) == 3 { 1 } else { 2 }));
            }
        }
        i += 1;
    }
    header(&name.rsplit('.').next().unwrap_or("").to_uppercase(), None, None)
}
