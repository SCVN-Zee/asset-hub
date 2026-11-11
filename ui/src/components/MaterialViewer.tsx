import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { DependencyReport, MaterialLook, StatsSummary } from '@shared/types'
import { parse } from '../parse'
import { FbxViewer, type PreviewShape } from '../viewer/FbxViewer'
import { fetchMaterialTextures, materialSlots, materialStats, type MaterialTexture } from '../viewer/materialPreview'
import type { LiveContext } from './ModelViewer'

interface Props {
  live: LiveContext
  /** Repo path of the .mat. */
  path: string
  report?: DependencyReport
  onLook(look: MaterialLook): void
  onStats(stats: StatsSummary): void
  onThumb(dataUrl: string): void
  onLoading?(loading: boolean): void
}

const SHAPES: PreviewShape[] = ['sphere', 'cube', 'plane']

/** A material on a sphere (or cube / plane) with its textures, colour and surface type. */
export function MaterialViewer({ live, path, report, onLook, onStats, onThumb, onLoading }: Props) {
  const { t } = useTranslation()
  const host = useRef<HTMLDivElement>(null)
  const viewer = useRef<FbxViewer>()
  const look = useRef<MaterialLook>()
  const matBytes = useRef(0)
  const textures = useRef<MaterialTexture[]>([])
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading')
  const [error, setError] = useState('')
  const [shape, setShape] = useState<PreviewShape>('sphere')
  const [autoRotate, setAutoRotate] = useState(false)
  const callbacks = useRef({ onLook, onStats, onThumb, onLoading })
  callbacks.current = { onLook, onStats, onThumb, onLoading }

  useEffect(() => {
    callbacks.current.onLoading?.(status === 'loading')
  }, [status])
  useEffect(() => () => callbacks.current.onLoading?.(false), [])

  useEffect(() => {
    const el = host.current
    if (!el) return
    let cancelled = false
    const v = new FbxViewer(el)
    viewer.current = v
    v.showShape('sphere')
    setShape('sphere')
    setAutoRotate(false)
    setStatus('loading')
    setError('')
    void (async () => {
      try {
        const data = await parse.material(live, path)
        if (cancelled) return
        matBytes.current = data.bytes
        look.current = data.look
        // Transparent materials are mostly effects (particles, decals): a flat card shows them as they are used.
        if (look.current.surface === 'transparent') {
          v.showShape('plane')
          setShape('plane')
        }
        v.setLook(look.current)
        callbacks.current.onLook(look.current)
        setStatus('ready')
      } catch (err) {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
        setStatus('error')
      }
    })()
    const own = textures.current
    return () => {
      cancelled = true
      v.dispose()
      viewer.current = undefined
      for (const tex of own) URL.revokeObjectURL(tex.file.url)
      own.length = 0
    }
  }, [live.repo, live.branch, live.root, path])

  // Textures follow the dependency report, which can arrive in steps (partial first).
  useEffect(() => {
    if (status !== 'ready' || !report) return
    const material = report.materials[0]
    let cancelled = false
    void (async () => {
      const owner = viewer.current
      const have = new Set(textures.current.map((x) => x.path))
      const wanted = { ...material, textures: material?.textures.filter((x) => x.path && !have.has(x.path)) ?? [] }
      const fetched = await fetchMaterialTextures(live, wanted)
      // The viewer closed or moved on while downloading: nobody will revoke these later.
      if (!owner || owner.isDisposed() || owner !== viewer.current) {
        fetched.forEach((x) => URL.revokeObjectURL(x.file.url))
        return
      }
      // An earlier step may have fetched the same file meanwhile.
      for (const x of fetched) {
        if (textures.current.some((y) => y.path === x.path)) URL.revokeObjectURL(x.file.url)
        else textures.current.push(x)
      }
      await owner.applyMaterialSlots(materialSlots(material, textures.current))
      if (look.current) owner.setLook(look.current)
      if (cancelled) return
      callbacks.current.onStats(materialStats(material, textures.current, matBytes.current, !report.partial))
      setTimeout(() => !owner.isDisposed() && callbacks.current.onThumb(owner.snapshot()), 300)
    })()
    return () => {
      cancelled = true
    }
  }, [status, report])

  return (
    <div className="viewer">
      <div className="viewer-canvas" ref={host} />
      {status !== 'ready' && (
        <div className="viewer-overlay" role="status">
          {status === 'loading' ? t('viewer.loading') : t('viewer.error', { message: error })}
        </div>
      )}
      {status === 'ready' && (
        <>
          <div className="viewer-tools" role="toolbar" aria-label={t('viewer.tools')}>
            {SHAPES.map((s) => (
              <button
                key={s}
                type="button"
                className="viewer-btn"
                aria-pressed={shape === s}
                onClick={() => {
                  viewer.current?.showShape(s)
                  setShape(s)
                }}
              >
                {t(`material.shape.${s}`)}
              </button>
            ))}
            <button
              type="button"
              className="viewer-btn"
              aria-pressed={autoRotate}
              onClick={() => {
                viewer.current?.setAutoRotate(!autoRotate)
                setAutoRotate(!autoRotate)
              }}
            >
              {t('viewer.autoRotate')}
            </button>
          </div>
          <span className="viewer-hint">{t('viewer.hint')}</span>
        </>
      )}
    </div>
  )
}
