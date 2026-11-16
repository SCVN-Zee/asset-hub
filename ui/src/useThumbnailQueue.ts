import { useCallback, useEffect, useRef, useState } from 'react'
import type { LiveContext } from './components/ModelViewer'
import type { StatsSummary } from '@shared/types'
import { createThumbTimer, thumbTimingEnabled, type ThumbTimer } from './viewer/thumbTiming'
export type ThumbKind = 'three' | 'material' | 'image' | 'audio' | 'prefab' | 'animation' | 'font' | 'tmpfont'

/** `elsewhere`: the asset is on another branch or repo than the open one (results of All branches / All repos). */
type Render = (ctx: LiveContext, path: string, elsewhere?: boolean, timer?: ThumbTimer) => Promise<{ url: string; stats?: StatsSummary; motion?: string }>

/** Renderers load with the first thumbnail of their kind (most draw with three.js, kept out of the startup bundle). */
const RENDER: Record<ThumbKind, () => Promise<Render>> = {
  three: () => import('./viewer/backgroundThumb').then((m) => m.renderThumbnail),
  material: () => import('./viewer/materialThumb').then((m) => m.renderMaterialThumbnail),
  image: () => import('./viewer/imageThumb').then((m) => m.renderImageThumbnail),
  audio: () => import('./viewer/audio').then((m) => m.renderAudioThumbnail),
  prefab: () => import('./viewer/prefabThumb').then((m) => m.renderPrefabThumbnail),
  animation: () => import('./viewer/animThumb').then((m) => m.renderAnimThumbnail),
  font: () => import('./viewer/font').then((m) => m.renderFontThumbnail),
  tmpfont: () => import('./viewer/tmpFont').then((m) => m.renderTmpFontThumbnail)
}

// Four at once draws about a fifth more per minute than two; more gains nothing (they wait on the disk and the GPU).
const CONCURRENCY = 4
/** A model whose thumbnail failed is retried after this long (the network may be back). */
const RETRY_MS = 2 * 60_000

interface Job {
  key: string
  path: string
  /** Position in the grid: the card nearest the top renders first. */
  order: number
  /** Repo + branch to read from, when not the open branch (results from other branches). */
  ctx?: LiveContext
  /** How it is drawn: a 3D model, a material on a sphere, a prefab put together, an image file as it is, or a clip's waveform. */
  kind: ThumbKind
  /** When it was first queued (kept when the same card is queued again), for the queue wait in timing logs. */
  queuedAt: number
}

/** Index of the job to start next: the priority one when it is queued, else the head (-1 when the queue is empty). */
export function takeNext(queue: readonly { key: string }[], priorityKey?: string): number {
  if (!queue.length) return -1
  const at = priorityKey === undefined ? -1 : queue.findIndex((j) => j.key === priorityKey)
  return at < 0 ? 0 : at
}

/** The queue without a cancelled card, unless it is the priority one (the selected card stays when it scrolls off screen). */
export function afterCancel<T extends { key: string }>(queue: readonly T[], key: string, priorityKey?: string): T[] {
  return key === priorityKey ? [...queue] : queue.filter((j) => j.key !== key)
}

/**
 * Renders thumbnails in the background for cards that are on screen, top of
 * the grid first. Cards that scroll away before their turn are dropped.
 * Nothing new starts while `paused` (the model the user opened is loading).
 */
export function useThumbnailQueue(live: LiveContext | undefined, onThumb: (key: string, dataUrl: string, stats?: StatsSummary, motion?: string) => void, paused = false) {
  const queue = useRef<Job[]>([])
  const active = useRef(new Set<string>())
  const failed = useRef(new Map<string, number>())
  /** The selected card: it starts before any other queued job, whatever order later requests give it. */
  const priorityKey = useRef<string>()
  const liveRef = useRef(live)
  const onThumbRef = useRef(onThumb)
  const pausedRef = useRef(paused)
  liveRef.current = live
  onThumbRef.current = onThumb
  pausedRef.current = paused
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set())

  const sync = () => setBusy(new Set([...active.current, ...queue.current.map((j) => j.key)]))

  const pump = useCallback(() => {
    while (!pausedRef.current && active.current.size < CONCURRENCY && queue.current.length) {
      const job = queue.current.splice(takeNext(queue.current, priorityKey.current), 1)[0]
      const ctx = job.ctx ?? liveRef.current
      if (!ctx) continue
      active.current.add(job.key)
      const timer = thumbTimingEnabled() ? createThumbTimer(undefined, job.queuedAt) : undefined
      timer?.mark('queue')
      void RENDER[job.kind]()
        .then((render) => render(ctx, job.path, !!job.ctx, job.kind === 'three' ? timer : undefined))
        .then(({ url, stats, motion }) => {
          onThumbRef.current(job.key, url, stats, motion)
          if (timer) console.info(timer.line(job.kind, job.path))
        })
        .catch(() => {
          failed.current.set(job.key, Date.now())
          if (timer) console.info(`${timer.line(job.kind, job.path)} failed-after=${timer.reached()}`)
        })
        .finally(() => {
          active.current.delete(job.key)
          sync()
          pump()
        })
    }
    sync()
  }, [])

  // Jobs belong to one repo + branch.
  useEffect(() => {
    queue.current = []
    sync()
  }, [live?.repo, live?.branch, live?.root])

  useEffect(() => {
    if (!paused) pump()
  }, [paused, pump])

  const request = useCallback(
    (key: string, path: string, order: number, ctx?: LiveContext, kind: ThumbKind = 'three') => {
      if (!(ctx ?? liveRef.current) || active.current.has(key) || Date.now() - (failed.current.get(key) ?? -Infinity) < RETRY_MS) return
      const queuedAt = queue.current.find((j) => j.key === key)?.queuedAt ?? performance.now()
      const rest = queue.current.filter((j) => j.key !== key)
      const at = rest.findIndex((j) => j.order > order)
      rest.splice(at < 0 ? rest.length : at, 0, { key, path, order, ctx, kind, queuedAt })
      queue.current = rest
      pump()
    },
    [pump]
  )

  const cancel = useCallback((key: string) => {
    const before = queue.current.length
    queue.current = afterCancel(queue.current, key, priorityKey.current)
    if (queue.current.length !== before) sync()
  }, [])

  /** Marks the selected card (undefined clears it); the caller still queues it with `request`. */
  const prioritize = useCallback((key: string | undefined) => {
    priorityKey.current = key
  }, [])

  /** Failed a short while ago: not retried yet, so not worth asking for. */
  const failedRecently = useCallback((key: string) => Date.now() - (failed.current.get(key) ?? -Infinity) < RETRY_MS, [])

  return { busy, request, cancel, prioritize, failedRecently }
}
