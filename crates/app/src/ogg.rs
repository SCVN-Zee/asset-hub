//! Ogg Vorbis to PCM WAV, for WebKit versions that cannot decode `.ogg` themselves.
//!
//! WebKit gained Ogg Vorbis in macOS 15.4 and the app runs on macOS 13, so the viewer asks for a
//! WAV only when `decodeAudioData` fails on an `.ogg`.

use std::io::Cursor;

use lewton::inside_ogg::OggStreamReader;

/// The WAV may not grow beyond this (a few minutes of CD-quality stereo is about 50 MB).
const MAX_WAV_BYTES: usize = 1024 * 1024 * 1024;

/// Decodes the whole stream to a 16-bit PCM WAV with the stream's channels and sample rate.
pub fn ogg_to_wav(ogg: &[u8]) -> Result<Vec<u8>, String> {
    let mut reader = OggStreamReader::new(Cursor::new(ogg)).map_err(|e| format!("not an Ogg Vorbis file: {e}"))?;
    let (channels, rate) = (u16::from(reader.ident_hdr.audio_channels), reader.ident_hdr.audio_sample_rate);
    if channels == 0 || rate == 0 {
        return Err("the Ogg stream has no audio format".into());
    }
    let mut pcm: Vec<i16> = Vec::new();
    // Interleaved samples, packet by packet.
    while let Some(packet) = reader.read_dec_packet_itl().map_err(|e| format!("could not decode the Ogg audio: {e}"))? {
        if (pcm.len() + packet.len()) * 2 > MAX_WAV_BYTES {
            return Err("the decoded audio is too large".into());
        }
        pcm.extend_from_slice(&packet);
    }
    Ok(wav(&pcm, channels, rate))
}

/// A canonical 44-byte-header PCM WAV of interleaved 16-bit samples.
fn wav(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0.25 s of a 440 Hz sine, 8 kHz stereo, encoded by ffmpeg's `vorbis` encoder:
    /// `ffmpeg -f lavfi -i sine=frequency=440:duration=0.25:sample_rate=8000 -ac 2 -c:a vorbis -strict -2 tone-stereo.ogg`
    const TONE: &[u8] = include_bytes!("testdata/tone-stereo.ogg");

    fn le16(wav: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([wav[at], wav[at + 1]])
    }

    fn le32(wav: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([wav[at], wav[at + 1], wav[at + 2], wav[at + 3]])
    }

    #[test]
    fn decodes_a_generated_ogg_to_a_wav_with_the_same_format_and_audible_samples() {
        let wav = ogg_to_wav(TONE).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(le16(&wav, 20), 1, "PCM");
        assert_eq!(le16(&wav, 22), 2, "channels");
        assert_eq!(le32(&wav, 24), 8000, "sample rate");
        assert_eq!(le16(&wav, 34), 16, "bits per sample");
        let data = le32(&wav, 40) as usize;
        assert_eq!(wav.len(), 44 + data);
        assert_eq!(le32(&wav, 4) as usize, wav.len() - 8);
        assert_eq!(data % 4, 0, "whole stereo frames");
        let frames = data / 4;
        assert!((1900..=2300).contains(&frames), "0.25 s at 8 kHz, got {frames} frames");
        let peak = wav[44..].as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b).unsigned_abs()).max().unwrap();
        assert!(peak > 2000, "the tone is not silent (peak {peak})");
    }

    #[test]
    fn the_wav_header_follows_the_channel_count_and_rate() {
        let wav = wav(&[1, -1, 2, -2], 1, 44100);
        assert_eq!((le16(&wav, 22), le32(&wav, 24), le32(&wav, 28), le16(&wav, 32)), (1, 44100, 88200, 2));
        assert_eq!(le32(&wav, 40), 8);
        assert_eq!(&wav[44..], [1, 0, 255, 255, 2, 0, 254, 255]);
    }

    #[test]
    fn refuses_what_is_not_ogg_vorbis_or_is_cut_short() {
        assert!(ogg_to_wav(b"RIFF....WAVE").unwrap_err().contains("not an Ogg Vorbis"));
        assert!(ogg_to_wav(&[]).is_err());
        assert!(ogg_to_wav(&TONE[..200]).is_err(), "the headers are cut off");
    }
}
