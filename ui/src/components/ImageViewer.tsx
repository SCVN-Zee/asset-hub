import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { SpriteBorder, SpriteImport, StatsSummary, TextureImport } from '@shared/types'
import { parse } from '../parse'
import { unwrap } from '../remoteSource'
import { builtSizeOf, decodeImage } from '../viewer/textures'
import type { LiveContext } from './ModelViewer'

/** What the texture panel shows about an image. */
export interface ImageInfo {
  width: number
  height: number
  bytes: number
  format: string
  hasAlpha: boolean
  /** Larger side after the import max size. */
  built: number
  /** Import settings from the .meta, when there is one. */
  import?: TextureImport
  /** Sprite settings, when the texture is imported as a sprite. */
  sprite?: SpriteImport
}

interface Props {
  live: LiveContext
  /** Repo path of the image. */
  path: string
  onInfo(info: ImageInfo): void
  onStats?(stats: StatsSummary): void
  onLoading?(loading: boolean): void
}

type Backdrop = 'checker' | 'dark' | 'light'

/** Something worth outlining: sheet rects or a 9-slice border. */
const hasOverlay = (s: SpriteImport | undefined) => !!s && (s.sprites.length > 0 || !!s.border)

/**
 * Sprite rects and 9-slice borders over the image. Unity measures rects from
 * the bottom-left corner; the canvas from the top-left.
 */
function drawSprites(g: CanvasRenderingContext2D, sprite: SpriteImport, width: number, height: number, scale: number) {
  const line = Math.max(1, scale)
  const slice = (x: number, y: number, w: number, h: number, b: SpriteBorder) => {
    g.save()
    g.strokeStyle = '#3ddc97'
    g.setLineDash([4 * line, 3 * line])
    g.beginPath()
    for (const vx of [x + b.left, x + w - b.right]) {
      if (vx <= x || vx >= x + w) continue
      g.moveTo(vx, y)
      g.lineTo(vx, y + h)
    }
    for (const vy of [y + b.top, y + h - b.bottom]) {
      if (vy <= y || vy >= y + h) continue
      g.moveTo(x, vy)
      g.lineTo(x + w, vy)
    }
    g.stroke()
    g.restore()
  }
  g.lineWidth = line
  if (sprite.border && !sprite.sprites.length) slice(0, 0, width, height, sprite.border)
  g.font = `${Math.round(11 * line)}px sans-serif`
  g.textBaseline = 'top'
  sprite.sprites.forEach((s, i) => {
    const top = height - s.y - s.height
    g.strokeStyle = '#ff5ca8'
    g.strokeRect(s.x + line / 2, top + line / 2, s.width - line, s.height - line)
    if (s.border) slice(s.x, top, s.width, s.height, s.border)
    // Numbered like the list in the panel.
    const label = String(i + 1)
    const pad = 3 * line
    g.fillStyle = 'rgba(255, 92, 168, 0.85)'
    g.fillRect(s.x, top, g.measureText(label).width + pad * 2, 11 * line + pad * 2)
    g.fillStyle = '#fff'
    g.fillText(label, s.x + pad, top + pad)
  })
}

/** A texture at full quality: fitted or at 1:1, over a checkerboard to show transparency, with sprite outlines. */
export function ImageViewer({ live, path, onInfo, onStats, onLoading }: Props) {
  const { t } = useTranslation()
  const canvas = useRef<HTMLCanvasElement>(null)
  const bitmap = useRef<ImageBitmap>()
  const [sprite, setSprite] = useState<SpriteImport>()
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading')
  const [error, setError] = useState('')
  const [actual, setActual] = useState(false)
  const [backdrop, setBackdrop] = useState<Backdrop>('checker')
  const [outlines, setOutlines] = useState(true)
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
    setActual(false)
    setSprite(undefined)
    void (async () => {
      try {
        const [bytes, info] = await Promise.all([window.api.assets.readFile(live.repo, live.branch, path).then(unwrap), parse.textureInfo(live, path)])
        if (cancelled) return
        const imp = info.import ?? undefined
        const img = await decodeImage(live, path, bytes, info, imp?.alphaSource)
        if (cancelled) return img.bitmap.close()
        // Kept until the next image, to redraw when outlines are toggled.
        bitmap.current = img.bitmap
        const el = canvas.current!
        el.width = img.width
        el.height = img.height
        const built = await builtSizeOf(live, path, info, img)
        if (cancelled) return
        const spriteInfo = info.sprite ?? undefined
        setSprite(spriteInfo)
        callbacks.current.onInfo({ width: img.width, height: img.height, bytes: bytes.byteLength, format: img.format, hasAlpha: img.hasAlpha, built, import: imp, sprite: spriteInfo })
        callbacks.current.onStats?.({
          triangles: 0,
          vertices: 0,
          bones: 0,
          clips: 0,
          materials: 0,
          fileBytes: 0,
          textureBytes: bytes.byteLength,
          maxTextureSize: built,
          width: img.width,
          height: img.height,
          source: 'viewer'
        })
        setStatus('ready')
      } catch (err) {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
        setStatus('error')
      }
    })()
    return () => {
      cancelled = true
      bitmap.current?.close()
      bitmap.current = undefined
    }
  }, [live.repo, live.branch, path])

  // The image, then outlines at a line width that stays readable however far the image is scaled down.
  useEffect(() => {
    const el = canvas.current
    const img = bitmap.current
    if (status !== 'ready' || !el || !img) return
    const draw = () => {
      const g = el.getContext('2d')!
      g.clearRect(0, 0, el.width, el.height)
      g.drawImage(img, 0, 0)
      if (outlines && sprite && hasOverlay(sprite)) drawSprites(g, sprite, el.width, el.height, el.width / Math.max(1, el.clientWidth))
    }
    draw()
    const resize = new ResizeObserver(draw)
    resize.observe(el)
    return () => resize.disconnect()
  }, [status, sprite, outlines, actual])

  return (
    <div className={`viewer image-viewer backdrop-${backdrop}`} data-actual={actual}>
      <div className="image-scroll">
        <canvas ref={canvas} className="image-canvas" />
      </div>
      {status !== 'ready' && (
        <div className="viewer-overlay" role="status">
          {status === 'loading' ? t('viewer.loading') : t('texture.cannotShow', { message: error })}
        </div>
      )}
      {status === 'ready' && (
        <div className="viewer-tools" role="toolbar" aria-label={t('viewer.tools')}>
          <button type="button" className="viewer-btn" aria-pressed={actual} onClick={() => setActual((a) => !a)}>
            {actual ? t('texture.fit') : t('texture.actual')}
          </button>
          {(['checker', 'dark', 'light'] as const).map((b) => (
            <button key={b} type="button" className="viewer-btn" aria-pressed={backdrop === b} onClick={() => setBackdrop(b)}>
              {t(`texture.backdrop.${b}`)}
            </button>
          ))}
          {sprite && hasOverlay(sprite) && (
            <button type="button" className="viewer-btn" aria-pressed={outlines} onClick={() => setOutlines((o) => !o)}>
              {sprite.sprites.length ? t('sprite.outlines') : t('sprite.slices')}
            </button>
          )}
        </div>
      )}
    </div>
  )
}
