import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { AnimClipInfo, DependencyReport } from '@shared/types'
import { parse } from '../parse'
import { ViewGizmo } from './ViewGizmo'
import { FbxViewer } from '../viewer/FbxViewer'
import { GOOD_ENOUGH, loadRig, pickRig, rememberRig, rigCoverage, rigOrder, THUMB_POSE, toThreeClip, type Rig } from '../viewer/animPlayer'
import { Dropdown } from './Dropdown'
import type { LiveContext } from './ModelViewer'

interface Props {
  live: LiveContext
  /** Repo path of the .anim. */
  path: string
  report?: DependencyReport
  onInfo(info: AnimClipInfo): void
  /** Every model of the branch (repo paths), to pick any as the rig. */
  models?: readonly string[]
  /** A snapshot on the rig chosen (automatically or by hand), to replace the grid thumbnail. */
  onThumb?(dataUrl: string): void
  onLoading?(loading: boolean): void
}

type Status = 'loading' | 'ready' | 'norig' | 'unsupported' | 'error'

/** Plays a clip on a model whose rig matches it: picked automatically from nearby models, or chosen by hand. */
export function AnimViewer({ live, path, report, models = [], onInfo, onThumb, onLoading }: Props) {
  const { t } = useTranslation()
  const host = useRef<HTMLDivElement>(null)
  const viewer = useRef<FbxViewer>()
  /** The rig on screen, to replay the clip on it. */
  const current = useRef<Rig>()
  /** The automatic choice runs once per clip; picking by hand does not start it again. */
  const autoPicked = useRef(false)
  const [info, setInfo] = useState<AnimClipInfo>()
  const [status, setStatus] = useState<Status>('loading')
  const [error, setError] = useState('')
  const [rig, setRig] = useState<string>()
  /** Rigs tried so far and how much of the clip each has. */
  const [coverage, setCoverage] = useState<Record<string, number>>({})
  const [playing, setPlaying] = useState(true)
  const [loop, setLoop] = useState(true)
  const callbacks = useRef({ onInfo, onThumb, onLoading })
  callbacks.current = { onInfo, onThumb, onLoading }

  useEffect(() => {
    callbacks.current.onLoading?.(status === 'loading')
  }, [status])
  useEffect(() => () => callbacks.current.onLoading?.(false), [])

  // The clip itself.
  useEffect(() => {
    const el = host.current
    if (!el) return
    const v = new FbxViewer(el)
    viewer.current = v
    let cancelled = false
    setInfo(undefined)
    setRig(undefined)
    current.current = undefined
    autoPicked.current = false
    setCoverage({})
    setStatus('loading')
    setPlaying(true)
    void parse
      .animClip(live, path)
      .then((clip) => {
        if (cancelled) return
        setInfo(clip)
        callbacks.current.onInfo(clip)
        if (clip.compressed || (clip.kind !== 'generic' && clip.kind !== 'legacy')) setStatus('unsupported')
      })
      .catch((err: unknown) => {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
        setStatus('error')
      })
    return () => {
      cancelled = true
      v.dispose()
      viewer.current = undefined
    }
  }, [live.repo, live.branch, path])

  const show = (r: Rig, clip: AnimClipInfo) => {
    const v = viewer.current
    if (!v) return
    current.current = r
    v.showGroup(r.root)
    // The grid shows the same pose as background thumbnails: the clip at 40%.
    const three = toThreeClip(clip, r)
    v.playClip(three, loop)
    v.poseAt(clip.length * THUMB_POSE)
    callbacks.current.onThumb?.(v.snapshot())
    v.playClip(three, loop)
    v.setPaused(false)
    setPlaying(true)
    setRig(r.path)
    setStatus('ready')
  }

  // Picks a rig: the nearby models in turn until one has most of the clip's objects.
  useEffect(() => {
    if (!info || status !== 'loading' || !report?.rigs || autoPicked.current) return
    autoPicked.current = true
    let cancelled = false
    let finished = false
    void (async () => {
      const best = await pickRig(live, info, rigOrder(live, path, report.rigs!), {
        isCancelled: () => cancelled,
        onTried: (p, coverage) => !cancelled && setCoverage((c) => ({ ...c, [p]: coverage }))
      })
      if (cancelled) return
      finished = true
      if (best) show(best.rig, info)
      else setStatus('norig')
    })()
    return () => {
      cancelled = true
      // Interrupted (the clip changed): the next run starts the search again.
      if (!finished) autoPicked.current = false
    }
    // Reports arrive in steps with the same rigs: compared by content.
  }, [info, report?.rigs?.join('|'), status])

  const [searching, setSearching] = useState(false)
  const chooseRig = async (p: string) => {
    if (!info) return
    setSearching(false)
    // Clips in the same folder use it too (and their thumbnails, when rendered).
    rememberRig(live, path, p)
    setStatus('loading')
    try {
      const r = await loadRig(live, p)
      setCoverage((c) => ({ ...c, [p]: rigCoverage(info, r.root) }))
      show(r, info)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      setStatus('error')
    }
  }

  const overlay =
    status === 'loading'
      ? t('anim.findingRig')
      : status === 'unsupported' && info
        ? t(info.compressed ? 'anim.compressed' : `anim.unsupported.${info.kind}`)
        : status === 'norig'
          ? t('anim.noRig')
          : status === 'error'
            ? t('viewer.error', { message: error })
            : undefined
  // A model that fits is the only one offered ("Other model…" still finds any). Without one: the
  // remembered model, then candidates not ruled out (tried with no matching bone).
  const fits = !!rig && (coverage[rig] ?? 0) >= GOOD_ENOUGH
  const rigs = fits ? [rig!] : rigOrder(live, path, report?.rigs ?? []).filter((p) => p === rig || coverage[p] === undefined || coverage[p] > 0)
  const OTHER = '__other'

  return (
    <div className="viewer">
      <div className="viewer-canvas" ref={host} />
      {overlay && (
        <div className="viewer-overlay" role="status">
          {overlay}
        </div>
      )}
      {status === 'ready' && <ViewGizmo viewer={viewer.current} />}
      {(status === 'ready' || status === 'norig' || (status === 'error' && !!info)) && (
        <div className="viewer-tools viewer-tools-top" role="toolbar" aria-label={t('viewer.tools')}>
          {status === 'ready' && (
            <>
              <button
                type="button"
                className="viewer-btn"
                aria-pressed={playing}
                onClick={() => {
                  viewer.current?.setPaused(playing)
                  setPlaying(!playing)
                }}
              >
                {playing ? t('audio.pause') : t('audio.play')}
              </button>
              <button
                type="button"
                className="viewer-btn"
                aria-pressed={loop}
                onClick={() => {
                  const next = !loop
                  setLoop(next)
                  // Restarts the clip with the new loop setting.
                  if (info && current.current) viewer.current?.playClip(toThreeClip(info, current.current), next)
                }}
              >
                {t('audio.loop')}
              </button>
              <button type="button" className="viewer-btn" onClick={() => viewer.current?.setView('reset')}>
                {t('viewer.view.reset')}
              </button>
            </>
          )}
          {searching ? (
            <input
              className="input viewer-select"
              list="anim-models"
              autoFocus
              placeholder={t('anim.searchModel')}
              aria-label={t('anim.searchModel')}
              onBlur={() => setSearching(false)}
              onKeyDown={(e) => e.key === 'Escape' && setSearching(false)}
              onChange={(e) => {
                const hit = models.find((m) => m === e.target.value)
                if (hit) void chooseRig(hit)
              }}
            />
          ) : (
            <Dropdown
              size="xs"
              className="viewer-select"
              label={t('anim.rig')}
              value={rig ?? ''}
              options={[
                ...(rig ? [] : [{ value: '', label: t('anim.pickRig') }]),
                ...(rig && !rigs.includes(rig) ? [{ value: rig, label: rig.split('/').pop() ?? rig }] : []),
                ...rigs.map((p) => ({ value: p, label: p.split('/').pop() ?? p, hint: coverage[p] !== undefined ? `${Math.round(coverage[p] * 100)}%` : undefined })),
                ...(models.length > 0 ? [{ value: OTHER, label: t('anim.otherModel') }] : [])
              ]}
              onChange={(value) => (value === OTHER ? setSearching(true) : void chooseRig(value))}
            />
          )}
          <datalist id="anim-models">{searching && models.map((m) => <option key={m} value={m} />)}</datalist>
        </div>
      )}
      {status === 'ready' && rig && <span className="viewer-hint">{t('anim.on', { name: rig.split('/').pop(), percent: Math.round((coverage[rig] ?? 0) * 100) })}</span>}
    </div>
  )
}
