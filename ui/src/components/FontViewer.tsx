import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { StatsSummary } from '@shared/types'
import { fontStats, loadFont, type LoadedFont } from '../viewer/font'
import type { LiveContext } from './ModelViewer'

interface Props {
  live: LiveContext
  /** Repo path of the font. */
  path: string
  onInfo(font: LoadedFont): void
  onStats?(stats: StatsSummary): void
  onLoading?(loading: boolean): void
}

/** Shown in the glyph grid: enough to judge the font without drawing a CJK font's thousands. */
const MAX_GLYPHS = 300

/** Sample text set in the font, editable, at a chosen size; then the characters it has. */
export function FontViewer({ live, path, onInfo, onStats, onLoading }: Props) {
  const { t } = useTranslation()
  const [font, setFont] = useState<LoadedFont>()
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading')
  const [error, setError] = useState('')
  const [text, setText] = useState(() => t('font.sample'))
  const [size, setSize] = useState(32)
  const callbacks = useRef({ onInfo, onStats, onLoading })
  callbacks.current = { onInfo, onStats, onLoading }

  useEffect(() => {
    callbacks.current.onLoading?.(status === 'loading')
  }, [status])
  useEffect(() => () => callbacks.current.onLoading?.(false), [])

  useEffect(() => {
    let cancelled = false
    let loaded: LoadedFont | undefined
    setStatus('loading')
    setError('')
    void loadFont(live, path).then(
      (f) => {
        if (cancelled) return f.release()
        loaded = f
        setFont(f)
        callbacks.current.onInfo(f)
        callbacks.current.onStats?.(fontStats(f))
        setStatus('ready')
      },
      (err: unknown) => {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
        setStatus('error')
      }
    )
    return () => {
      cancelled = true
      loaded?.release()
      setFont(undefined)
    }
  }, [live.repo, live.branch, path])

  const glyphs = useMemo(() => (font ? [...font.codepoints].filter((c) => c > 0x20 && (c < 0x7f || c > 0x9f)).slice(0, MAX_GLYPHS) : []), [font])
  const family = font ? `"${font.css}"` : undefined
  /** Characters the font lacks would silently fall back to another font: they are marked instead. */
  const sample = [...(text || t('font.sample'))]

  return (
    <div className="viewer font-viewer">
      {status !== 'ready' || !font ? (
        <div className="viewer-overlay" role="status">
          {status === 'error' ? t('font.cannotRead', { message: error }) : t('viewer.loading')}
        </div>
      ) : (
        <>
          <div className="font-tools">
            <input className="input" value={text} onChange={(e) => setText(e.target.value)} aria-label={t('font.sampleLabel')} placeholder={t('font.sampleLabel')} />
            <input type="range" min={12} max={96} value={size} onChange={(e) => setSize(Number(e.target.value))} aria-label={t('font.size')} />
            <span className="muted small mono">{size}px</span>
          </div>
          <p className="font-sample" style={{ fontFamily: family, fontSize: size }}>
            {sample.map((ch, i) => {
              const c = ch.codePointAt(0)!
              return c <= 0x20 || font.codepoints.has(c) ? ch : (
                <span key={i} className="font-missing" title={t('font.missingChar', { char: ch })}>
                  {ch}
                </span>
              )
            })}
          </p>
          <div className="font-glyphs" style={{ fontFamily: family }} aria-label={t('font.glyphs')}>
            {glyphs.map((c) => (
              <span key={c} title={`U+${c.toString(16).toUpperCase().padStart(4, '0')}`}>
                {String.fromCodePoint(c)}
              </span>
            ))}
          </div>
          {font.codepoints.size > glyphs.length && <p className="muted small font-more">{t('font.moreGlyphs', { count: font.codepoints.size - glyphs.length })}</p>}
        </>
      )}
    </div>
  )
}
