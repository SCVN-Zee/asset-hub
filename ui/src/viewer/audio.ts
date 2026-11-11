import type { AudioImport, AudioImportSettings, StatsSummary } from '@shared/types'
import { parse } from '../parse'
import type { LiveContext } from '../components/ModelViewer'
import { unwrap } from '../remoteSource'
import { FLAT_THUMB_VERSION } from './thumbVersions'

/** A decoded clip with what the audio panel reports about it. */
export interface DecodedAudio {
  duration: number
  /** Of the file (Web Audio resamples what it decodes). */
  sampleRate: number
  channels: number
  format: string
  bytes: number
  /** Min / max per column across all channels, for the waveform. */
  peaks: Float32Array
  import?: AudioImport
  /** What the clip takes in a build (Rust works it out from the length, the format and the import). */
  builtBytes: number
  /** The settings a build takes, from the import (an override for a platform, or the import itself). */
  effective?: AudioImportSettings
  /** The clip as a WAV, when the web view cannot read the file's own format (Ogg Vorbis on older macOS): what to play. */
  wav?: Uint8Array
}

const COLUMNS = 800
let decoder: OfflineAudioContext | undefined

/**
 * Reads a clip's samples. The web view's decoder is tried first; when it fails on an Ogg Vorbis file
 * (WebKit before macOS 14 cannot read it), the Rust side converts it to a WAV, which is then decoded.
 */
async function decodeSamples(bytes: Uint8Array, format: string): Promise<{ buffer: AudioBuffer; wav?: Uint8Array }> {
  decoder ??= new OfflineAudioContext(1, 1, 44100)
  try {
    return { buffer: await decoder.decodeAudioData(bytes.slice().buffer) }
  } catch (error) {
    if (format !== 'OGG') throw error
    const wav = await parse.oggToWav(bytes)
    return { buffer: await decoder.decodeAudioData(wav.slice().buffer), wav }
  }
}

/** Decodes a clip and reduces it to waveform peaks; the samples themselves are not kept. */
export async function decodeAudio(live: LiveContext, path: string, bytes: Uint8Array, background = false): Promise<DecodedAudio> {
  const info = await parse.audioInfo(live, path, background)
  const { header } = info
  const { buffer, wav } = await decodeSamples(bytes, header.format)
  const peaks = new Float32Array(COLUMNS * 2)
  const step = Math.max(1, Math.floor(buffer.length / COLUMNS))
  for (let c = 0; c < buffer.numberOfChannels; c++) {
    const data = buffer.getChannelData(c)
    for (let col = 0; col < COLUMNS; col++) {
      let min = peaks[col * 2]
      let max = peaks[col * 2 + 1]
      for (let i = col * step, end = Math.min(data.length, i + step); i < end; i++) {
        if (data[i] < min) min = data[i]
        if (data[i] > max) max = data[i]
      }
      peaks[col * 2] = min
      peaks[col * 2 + 1] = max
    }
  }
  const sampleRate = header.sampleRate ?? buffer.sampleRate
  const channels = header.channels ?? buffer.numberOfChannels
  const settings = info.import ?? undefined
  return {
    duration: buffer.duration,
    sampleRate,
    channels,
    format: header.format,
    bytes: bytes.byteLength,
    peaks,
    import: settings,
    effective: info.effective ?? undefined,
    builtBytes: (await parse.audioBuiltBytes(buffer.duration, sampleRate, channels, settings)) ?? 0,
    ...(wav ? { wav } : {})
  }
}

/** Waveform ink and playhead colour: readable on the grey stage (#767676) the viewer and the cards sit on. */
export const WAVE_INK = '#1f1f20'
export const WAVE_PLAYED = '#ffffff'

/** Draws the waveform; columns left of `progress` (0–1) in the played colour. */
export function drawWaveform(canvas: HTMLCanvasElement, peaks: Float32Array, colors: { wave: string; played?: string; background?: string }, progress = 0): void {
  const g = canvas.getContext('2d')!
  const { width: w, height: h } = canvas
  if (colors.background) {
    g.fillStyle = colors.background
    g.fillRect(0, 0, w, h)
  } else g.clearRect(0, 0, w, h)
  const columns = peaks.length / 2
  const mid = h / 2
  for (let x = 0; x < w; x++) {
    const col = Math.floor((x / w) * columns)
    const min = peaks[col * 2]
    const max = peaks[col * 2 + 1]
    g.fillStyle = colors.played && x / w < progress ? colors.played : colors.wave
    const top = mid - max * mid * 0.95
    g.fillRect(x, top, 1, Math.max(1, (max - min) * mid * 0.95))
  }
}

export function audioStats(a: DecodedAudio): StatsSummary {
  return {
    triangles: 0,
    vertices: 0,
    bones: 0,
    clips: 0,
    materials: 0,
    fileBytes: Math.round(a.builtBytes),
    textureBytes: 0,
    maxTextureSize: 0,
    duration: a.duration,
    source: 'viewer'
  }
}

/** Grid thumbnail of a clip: its waveform, plus its length and estimated built size. */
export async function renderAudioThumbnail(live: LiveContext, path: string): Promise<{ url: string; stats: StatsSummary }> {
  const bytes = await window.api.assets.readFile(live.repo, live.branch, path, true).then(unwrap)
  const audio = await decodeAudio(live, path, bytes, true)
  const canvas = document.createElement('canvas')
  canvas.width = 320
  canvas.height = 240
  // No background: the card's stage shows through.
  drawWaveform(canvas, audio.peaks, { wave: WAVE_INK })
  return { url: canvas.toDataURL('image/webp', 0.85), stats: { ...audioStats(audio), modelThumb: FLAT_THUMB_VERSION } }
}

/** 75.4 → "1:15". */
export function formatDuration(seconds: number): string {
  if (seconds < 10) return `${seconds.toFixed(1)} s`
  const s = Math.round(seconds)
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}
