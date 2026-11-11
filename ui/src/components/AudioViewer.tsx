import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { StatsSummary } from '@shared/types'
import { unwrap } from '../remoteSource'
import { audioStats, decodeAudio, drawWaveform, formatDuration, WAVE_INK, WAVE_PLAYED, type DecodedAudio } from '../viewer/audio'
import type { LiveContext } from './ModelViewer'

interface Props {
  live: LiveContext
  /** Repo path of the clip. */
  path: string
  onInfo(info: DecodedAudio): void
  onStats?(stats: StatsSummary): void
  onLoading?(loading: boolean): void
}

const MIME: Record<string, string> = { WAV: 'audio/wav', OGG: 'audio/ogg', MP3: 'audio/mpeg' }

const AUTO_PLAY_KEY = 'assethub.audio.autoPlay'

/** Remembered per machine; storage can be unavailable, then it is just off. */
function readAutoPlay(): boolean {
  try {
    return localStorage.getItem(AUTO_PLAY_KEY) === '1'
  } catch {
    return false
  }
}

function writeAutoPlay(on: boolean): void {
  try {
    localStorage.setItem(AUTO_PLAY_KEY, on ? '1' : '0')
  } catch {
    // Not remembered: still applies until the app closes.
  }
}

/** Plays a clip over its waveform: click to seek, space to play / pause, optional loop. */
export function AudioViewer({ live, path, onInfo, onStats, onLoading }: Props) {
  const { t } = useTranslation()
  const canvas = useRef<HTMLCanvasElement>(null)
  const audio = useRef<HTMLAudioElement>(null)
  const peaks = useRef<Float32Array>()
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading')
  const [error, setError] = useState('')
  const [url, setUrl] = useState<string>()
  const [playing, setPlaying] = useState(false)
  const [loop, setLoop] = useState(false)
  /** Plays each clip as soon as it is selected, so browsing sounds needs no click. */
  const [autoPlay, setAutoPlay] = useState(readAutoPlay)
  const [time, setTime] = useState(0)
  const [duration, setDuration] = useState(0)
  const callbacks = useRef({ onInfo, onStats, onLoading })
  callbacks.current = { onInfo, onStats, onLoading }

  useEffect(() => {
    callbacks.current.onLoading?.(status === 'loading')
  }, [status])
  useEffect(() => () => callbacks.current.onLoading?.(false), [])

  const redraw = (progress: number) => {
    const el = canvas.current
    if (!el || !peaks.current) return
    const ratio = Math.min(window.devicePixelRatio, 2)
    const w = Math.max(1, Math.round(el.clientWidth * ratio))
    const h = Math.max(1, Math.round(el.clientHeight * ratio))
    if (el.width !== w || el.height !== h) {
      el.width = w
      el.height = h
    }
    drawWaveform(el, peaks.current, { wave: WAVE_INK, played: WAVE_PLAYED }, progress)
  }

  useEffect(() => {
    let cancelled = false
    let blobUrl: string | undefined
    setStatus('loading')
    setError('')
    setPlaying(false)
    setTime(0)
    peaks.current = undefined
    void (async () => {
      try {
        const bytes = await window.api.assets.readFile(live.repo, live.branch, path).then(unwrap)
        if (cancelled) return
        const decoded = await decodeAudio(live, path, bytes)
        if (cancelled) return
        peaks.current = decoded.peaks
        // A clip the web view could not read itself plays as the WAV the Rust side made of it.
        blobUrl = URL.createObjectURL(decoded.wav ? new Blob([decoded.wav as Uint8Array<ArrayBuffer>], { type: 'audio/wav' }) : new Blob([bytes as Uint8Array<ArrayBuffer>], { type: MIME[decoded.format] ?? 'application/octet-stream' }))
        setUrl(blobUrl)
        setDuration(decoded.duration)
        callbacks.current.onInfo(decoded)
        callbacks.current.onStats?.(audioStats(decoded))
        setStatus('ready')
      } catch (err) {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
        setStatus('error')
      }
    })()
    return () => {
      cancelled = true
      if (blobUrl) URL.revokeObjectURL(blobUrl)
      setUrl(undefined)
    }
  }, [live.repo, live.branch, path])

  // A new clip is ready (the <audio> element now has its source): start it when auto play is on.
  useEffect(() => {
    if (url && autoPlay) void audio.current?.play().catch((err: unknown) => setError(err instanceof Error ? err.message : String(err)))
    // Only on a new clip: turning auto play on does not start the current one.
  }, [url])

  // The playhead moves smoothly while playing; the waveform follows panel resizes.
  useEffect(() => {
    if (status !== 'ready') return
    let frame = 0
    const tick = () => {
      const a = audio.current
      if (a) {
        setTime(a.currentTime)
        redraw(duration ? a.currentTime / duration : 0)
      }
      if (playing) frame = requestAnimationFrame(tick)
    }
    tick()
    const resize = new ResizeObserver(() => redraw(duration && audio.current ? audio.current.currentTime / duration : 0))
    if (canvas.current) resize.observe(canvas.current)
    return () => {
      cancelAnimationFrame(frame)
      resize.disconnect()
    }
  }, [status, playing, duration])

  const toggle = () => {
    const a = audio.current
    if (!a) return
    setError('')
    if (a.paused) void a.play().catch((err: unknown) => setError(err instanceof Error ? err.message : String(err)))
    else a.pause()
  }

  const seek = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const a = audio.current
    if (!a || !duration) return
    const box = e.currentTarget.getBoundingClientRect()
    a.currentTime = Math.min(duration, Math.max(0, ((e.clientX - box.left) / box.width) * duration))
    setTime(a.currentTime)
    redraw(a.currentTime / duration)
  }

  return (
    <div
      className="viewer audio-viewer"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === ' ' && status === 'ready') {
          e.preventDefault()
          toggle()
        }
      }}
    >
      <canvas ref={canvas} className="audio-wave" onClick={seek} aria-label={t('audio.seek')} />
      {url && <audio ref={audio} src={url} loop={loop} onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onEnded={() => setPlaying(false)} onError={() => setError(audio.current?.error?.message || t('audio.unsupported'))} />}
      {status !== 'ready' && (
        <div className="viewer-overlay" role="status">
          {status === 'loading' ? t('viewer.loading') : t('audio.cannotPlay', { message: error })}
        </div>
      )}
      {status === 'ready' && (
        <div className="viewer-tools" role="toolbar" aria-label={t('viewer.tools')}>
          <button type="button" className="viewer-btn" aria-pressed={playing} onClick={toggle}>
            {playing ? t('audio.pause') : t('audio.play')}
          </button>
          <button type="button" className="viewer-btn" aria-pressed={loop} onClick={() => setLoop((l) => !l)}>
            {t('audio.loop')}
          </button>
          <button
            type="button"
            className="viewer-btn"
            aria-pressed={autoPlay}
            title={t('audio.autoPlayHint')}
            onClick={() => {
              writeAutoPlay(!autoPlay)
              setAutoPlay(!autoPlay)
            }}
          >
            {t('audio.autoPlay')}
          </button>
          {error && (
            <span className="audio-error" role="alert">
              {t('audio.cannotPlay', { message: error })}
            </span>
          )}
          <span className="audio-time mono">
            {formatDuration(time)} / {formatDuration(duration)}
          </span>
        </div>
      )}
    </div>
  )
}
