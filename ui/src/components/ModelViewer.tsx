import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { DependencyReport, MaterialRef, ModelStats } from '@shared/types'
import { parse } from '../parse'
import { unwrap } from '../remoteSource'
import { Dropdown } from './Dropdown'
import { ViewGizmo } from './ViewGizmo'
import { FbxViewer, type MaterialSlot, type TextureFile } from '../viewer/FbxViewer'
import { DRAWABLE, fetchTextureFile, guessedSlots, materialSlotPaths, reportSlotPaths } from '../viewer/textures'

export interface LiveContext {
  repo: string
  branch: string
  /** Unity project root prefix inside the repo. */
  root: string
}

interface Props {
  live: LiveContext
  /** Repo path of the FBX. */
  path: string
  report?: DependencyReport
  /** Material a prefab puts on this model, applied to every mesh (for FBX without their own). */
  prefabMaterial?: MaterialRef
  onStats(stats: ModelStats): void
  onThumb(dataUrl: string): void
  /** True while the model is downloading / loading, so background work can wait. */
  onLoading?(loading: boolean): void
  /** Starts the first animation clip once loaded (when browsing animations). */
  autoPlay?: boolean
}

const MAX_TEXTURES = 16

type Status = 'loading' | 'ready' | 'error'

export function ModelViewer({ live, path, report, prefabMaterial, onStats, onThumb, onLoading, autoPlay }: Props) {
  const { t } = useTranslation()
  const host = useRef<HTMLDivElement>(null)
  const viewer = useRef<FbxViewer>()
  const files = useRef(new Map<string, TextureFile>())
  const fileBytes = useRef(0)
  const [status, setStatus] = useState<Status>('loading')
  const [error, setError] = useState('')
  const [autoRotate, setAutoRotate] = useState(false)
  const [wireframe, setWireframe] = useState(false)
  const [clips, setClips] = useState<string[]>([])
  const [clip, setClip] = useState('')
  const autoPlayRef = useRef(autoPlay)
  autoPlayRef.current = autoPlay
  const callbacks = useRef({ onStats, onThumb, onLoading })
  callbacks.current = { onStats, onThumb, onLoading }
  useEffect(() => {
    callbacks.current.onLoading?.(status === 'loading')
  }, [status])
  // Closing the viewer mid-load must not leave background work paused.
  useEffect(() => () => callbacks.current.onLoading?.(false), [])

  /** Downloads texture files once per viewer and serves them as blob URLs. */
  const fetchTextures = async (paths: readonly string[]) => {
    const owner = viewer.current
    const wanted = [...new Set(paths)].filter((p) => DRAWABLE.test(p) && !files.current.has(p)).slice(0, MAX_TEXTURES)
    await Promise.all(
      wanted.map(async (p) => {
        const file = await fetchTextureFile(live, p)
        if (!file) return
        // The viewer closed (or moved to another model) while downloading: nobody will revoke this URL later.
        if (!owner || owner.isDisposed() || owner !== viewer.current) URL.revokeObjectURL(file.url)
        else files.current.set(p, file)
      })
    )
  }

  const publish = () => {
    const v = viewer.current
    if (!v) return
    callbacks.current.onStats(v.stats(fileBytes.current, [...files.current.values()]))
    setTimeout(() => !v.isDisposed() && callbacks.current.onThumb(v.snapshot()), 300)
  }

  useEffect(() => {
    const el = host.current
    if (!el) return
    let cancelled = false
    const v = new FbxViewer(el)
    viewer.current = v
    const ownFiles = files.current
    setStatus('loading')
    setError('')
    setClips([])
    setClip('')
    setAutoRotate(false)
    setWireframe(false)
    void (async () => {
      try {
        const bytes = unwrap(await window.api.assets.readFile(live.repo, live.branch, path))
        if (cancelled) return
        fileBytes.current = bytes.byteLength
        const names = await parse.fbxTextureNames(live, path)
        const found = names.length ? unwrap(await window.api.assets.findByNames(live.repo, live.branch, names, path, live.root)) : {}
        await fetchTextures(Object.values(found).filter((p): p is string => !!p))
        if (cancelled) return
        const byName = new Map([...ownFiles.values()].map((f) => [f.name.toLowerCase(), f]))
        await v.load(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer, byName)
        if (cancelled) return
        // Textures named in the FBX but not wired to a slot: place them by file name.
        if (!v.hasColorMaps() && byName.size) await v.applyMaterialSlots(guessedSlots([...byName.values()]))
        if (cancelled) return
        const clipList = v.stats(0, []).animations
        setClips(clipList)
        setStatus('ready')
        // The thumbnail is taken first, in the bind pose like other models; then the clip starts.
        publish()
        if (autoPlayRef.current && clipList.length)
          setTimeout(() => {
            if (cancelled || v.isDisposed()) return
            v.playAnimation(0)
            setClip('0')
          }, 400)
      } catch (err) {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
        setStatus('error')
      }
    })()
    return () => {
      cancelled = true
      v.dispose()
      viewer.current = undefined
      for (const f of ownFiles.values()) URL.revokeObjectURL(f.url)
      ownFiles.clear()
    }
    // live is compared by value through its fields.
  }, [live.repo, live.branch, live.root, path])

  // Once materials are known (kind A .mat files, or one found on a prefab), put their textures on the model.
  useEffect(() => {
    if (status !== 'ready') return
    const textures = [...reportSlotPaths(report), ...(prefabMaterial ? materialSlotPaths(prefabMaterial, '*') : [])]
    if (!textures.length) return
    let cancelled = false
    void (async () => {
      await fetchTextures(textures.map((tex) => tex.path))
      if (cancelled || !viewer.current) return
      const slots: MaterialSlot[] = textures.flatMap((tex) => {
        const file = files.current.get(tex.path)
        return file ? [{ material: tex.material, prop: tex.prop, file }] : []
      })
      await viewer.current.applyMaterialSlots(slots)
      if (!cancelled) publish()
    })()
    return () => {
      cancelled = true
    }
  }, [status, report, prefabMaterial])

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
          <ViewGizmo viewer={viewer.current} />
          <div className="viewer-tools viewer-tools-top" role="toolbar" aria-label={t('viewer.tools')}>
            <button type="button" className="viewer-btn" onClick={() => viewer.current?.setView('reset')}>
              {t('viewer.view.reset')}
            </button>
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
            <button
              type="button"
              className="viewer-btn"
              aria-pressed={wireframe}
              onClick={() => {
                viewer.current?.setWireframe(!wireframe)
                setWireframe(!wireframe)
              }}
            >
              {t('viewer.wireframe')}
            </button>
          </div>
          {clips.length > 0 && (
            <div className="viewer-clips">
              <Dropdown
                size="xs"
                className="viewer-select"
                label={t('viewer.animation')}
                value={clip}
                options={[{ value: '', label: t('viewer.noAnimation', { count: clips.length }) }, ...clips.map((c, i) => ({ value: String(i), label: `▶ ${c}` }))]}
                onChange={(value) => {
                  setClip(value)
                  viewer.current?.playAnimation(value === '' ? null : Number(value))
                }}
              />
            </div>
          )}
          <span className="viewer-hint">{t('viewer.hint')}</span>
        </>
      )}
    </div>
  )
}
