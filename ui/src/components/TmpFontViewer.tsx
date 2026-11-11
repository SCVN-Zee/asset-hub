import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { StatsSummary } from '@shared/types'
import { loadTmpFont, tmpFontStats, type LoadedTmpFont } from '../viewer/tmpFont'
import type { LiveContext } from './ModelViewer'

interface Props {
  live: LiveContext
  /** Repo path of the .asset. */
  path: string
  onInfo(font: LoadedTmpFont): void
  onThumb?(dataUrl: string): void
  onStats?(stats: StatsSummary): void
  onLoading?(loading: boolean): void
}

/** A TextMeshPro font asset: its atlas, as glyphs in white on the viewer's dark background. */
export function TmpFontViewer({ live, path, onInfo, onStats, onLoading }: Props) {
  const { t } = useTranslation()
  const [font, setFont] = useState<LoadedTmpFont>()
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading')
  const [error, setError] = useState('')
  const callbacks = useRef({ onInfo, onStats, onLoading })
  callbacks.current = { onInfo, onStats, onLoading }

  useEffect(() => {
    callbacks.current.onLoading?.(status === 'loading')
  }, [status])
  useEffect(() => () => callbacks.current.onLoading?.(false), [])

  useEffect(() => {
    let cancelled = false
    setStatus('loading')
    setError('')
    setFont(undefined)
    void loadTmpFont(live, path).then(
      (f) => {
        if (cancelled) return
        setFont(f)
        callbacks.current.onInfo(f)
        callbacks.current.onStats?.(tmpFontStats(f))
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
    }
  }, [live.repo, live.branch, path])

  return (
    <div className="viewer tmp-viewer">
      {status === 'ready' && font?.atlas ? (
        <img className="tmp-atlas" src={font.atlas} alt={t('tmpFont.atlas')} />
      ) : (
        <div className="viewer-overlay" role="status">
          {status === 'loading' ? t('viewer.loading') : status === 'error' ? t('font.cannotRead', { message: error }) : t('tmpFont.noAtlas')}
        </div>
      )}
    </div>
  )
}
