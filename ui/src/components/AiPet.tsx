import { Fragment, useEffect, useRef, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { AiStatus } from '@shared/ipc'
import { reportAnswer, resetChat, sendChat, stopChat, useAiChat, type ChatMessage } from '../aiChat'
import { petExpression } from '../petFace'
import { PixelFace } from './PixelFace'

/** **bold** and `code` inside a line; everything else stays plain text (never HTML). */
function inline(text: string): ReactNode[] {
  return text.split(/(\*\*[^*]+\*\*|`[^`]+`)/g).map((part, i) =>
    part.startsWith('**') && part.endsWith('**') && part.length > 4 ? (
      <strong key={i}>{part.slice(2, -2)}</strong>
    ) : part.startsWith('`') && part.endsWith('`') && part.length > 2 ? (
      <code key={i}>{part.slice(1, -1)}</code>
    ) : (
      <Fragment key={i}>{part}</Fragment>
    )
  )
}

/** A short answer: lines, with "- " lines as bullets. */
function Answer({ text }: { text: string }) {
  const lines = text.split('\n').filter((l) => l.trim())
  return (
    <>
      {lines.map((line, i) =>
        /^\s*[-*•]\s+/.test(line) ? (
          <span key={i} className="ai-bullet">
            {inline(line.replace(/^\s*[-*•]\s+/, ''))}
          </span>
        ) : (
          <span key={i} className="ai-line">
            {inline(line)}
          </span>
        )
      )}
    </>
  )
}

function Choices({ choices, onPick, onOther }: { choices: NonNullable<ChatMessage['choices']>; onPick(text: string): void; onOther(): void }) {
  const { t } = useTranslation()
  return (
    <div className="ai-choices" role="group" aria-label={t('aiPet.choices')}>
      {choices.options.map((o) => (
        <button key={o} type="button" className="ai-choice" onClick={() => onPick(o)}>
          {o}
        </button>
      ))}
      {choices.other && (
        <button type="button" className="ai-choice ai-choice-other" onClick={onOther}>
          {t('aiPet.other')}
        </button>
      )}
    </div>
  )
}

/** The new-answer badge, an 8×8 round LED matrix like the face: [x, y, lit]. */
const BADGE_DOTS: Array<[number, number, boolean]> = []
for (let y = 0; y < 8; y++)
  for (let x = 0; x < 8; x++)
    // "…": a message waiting, three 2×2 dots across the middle.
    if ((x - 3.5) ** 2 + (y - 3.5) ** 2 <= 4.2 ** 2) BADGE_DOTS.push([x, y, (y === 3 || y === 4) && x % 3 !== 2])

/** Pictures attached to one message, at most. */
const MAX_IMAGES = 4
/** Longest side an attached picture is sent at (larger ones are scaled down: the model sees about this much). */
const IMAGE_SIDE = 1568

/** An image file as a data URL the AI can read: scaled down to `IMAGE_SIDE`, as WebP (keeps transparency). */
async function imageData(file: Blob): Promise<string | undefined> {
  try {
    const bitmap = await createImageBitmap(file)
    const k = Math.min(1, IMAGE_SIDE / Math.max(bitmap.width, bitmap.height))
    const canvas = document.createElement('canvas')
    canvas.width = Math.max(1, Math.round(bitmap.width * k))
    canvas.height = Math.max(1, Math.round(bitmap.height * k))
    canvas.getContext('2d')!.drawImage(bitmap, 0, 0, canvas.width, canvas.height)
    bitmap.close()
    return canvas.toDataURL('image/webp', 0.9)
  } catch {
    return undefined
  }
}

/** A reaction (found, error, sad) shows this long once the bubble is closed. */
const REACTION_MS = 8000
/** The pet dozes off after this long without anything happening. */
const DOZE_MS = 5 * 60_000

/**
 * The AI assistant as a small pet in the hub's corner: a round LED face whose
 * expression follows the chat (see petFace), opening a chat bubble for short
 * exchanges. Shown once ChatGPT is connected, unless turned off in Settings.
 */
export function AiPet({ language, enabled }: { language: string; enabled: boolean }) {
  const { t } = useTranslation()
  const { messages, busy, activity } = useAiChat()
  const [status, setStatus] = useState<AiStatus>()
  const [open, setOpen] = useState(false)
  const [draft, setDraft] = useState('')
  /** Pictures waiting to go with the next message. */
  const [images, setImages] = useState<string[]>([])
  const [dragging, setDragging] = useState(false)
  const picker = useRef<HTMLInputElement>(null)
  const log = useRef<HTMLDivElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const root = useRef<HTMLDivElement>(null)
  /** An answer came in while the bubble was closed: a red "!" on the avatar until it is opened. */
  const [unseen, setUnseen] = useState(false)
  // When the latest answer came in (or was reported), and when anything last happened: they set the face.
  const [reactedAt, setReactedAt] = useState(0)
  const [activeAt, setActiveAt] = useState(() => Date.now())
  const [, setNow] = useState(0)
  const last = [...messages].reverse().find((m) => m.role === 'assistant')
  useEffect(() => {
    if (!busy && last && last.state !== 'pending') {
      setReactedAt(Date.now())
      if (!open && !last.reported) setUnseen(true)
    }
  }, [busy, last?.id, last?.reported])
  useEffect(() => {
    if (open) setUnseen(false)
  }, [open])
  // A press anywhere outside the pet folds the bubble away (the conversation stays).
  useEffect(() => {
    if (!open) return
    const outside = (e: PointerEvent) => {
      if (root.current && !root.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('pointerdown', outside)
    return () => document.removeEventListener('pointerdown', outside)
  }, [open])
  useEffect(() => setActiveAt(Date.now()), [open, draft, busy, messages.length])
  // Re-checks the face once a reaction is over and once it is time to doze.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 2000)
    return () => window.clearInterval(timer)
  }, [])

  useEffect(() => {
    void window.api.ai.status().then(setStatus)
    return window.api.ai.onStatus(setStatus)
  }, [])
  // To the newest line, including reply buttons, which appear when the answer is done (busy turns false).
  useEffect(() => {
    log.current?.scrollTo({ top: log.current.scrollHeight })
  }, [messages, open, busy])
  useEffect(() => {
    if (open) input.current?.focus()
  }, [open])
  // ⌘J opens and closes the bubble from anywhere.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'j') {
        e.preventDefault()
        setOpen((o) => !o)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  if (!enabled || status?.state !== 'connected') return null

  const send = () => {
    if ((!draft.trim() && !images.length) || busy) return
    void sendChat(draft, language, images)
    setDraft('')
    setImages([])
  }
  /** Adds image files (picked, pasted or dropped), up to `MAX_IMAGES`. */
  const attach = async (files: Iterable<File>) => {
    const list = [...files].filter((f) => f.type.startsWith('image/')).slice(0, MAX_IMAGES)
    if (!list.length) return false
    const urls = (await Promise.all(list.map(imageData))).filter((u): u is string => !!u)
    setImages((prev) => [...prev, ...urls].slice(0, MAX_IMAGES))
    input.current?.focus()
    return true
  }

  const now = Date.now()
  const expression = petExpression({
    open,
    typing: !!draft.trim() || images.length > 0,
    busy,
    working: !!activity,
    last: last && last.state !== 'pending' ? { state: last.state, reported: last.reported, choices: !!last.choices, text: !!last.text } : undefined,
    fresh: now - reactedAt < REACTION_MS,
    dozing: now - activeAt > DOZE_MS
  })

  return (
    <div ref={root} className="ai-pet" data-busy={busy || undefined} onKeyDown={(e) => e.key === 'Escape' && setOpen(false)}>
      {open && (
        <div
          className="ai-pet-bubble"
          role="dialog"
          aria-label={t('aiPet.title')}
          data-dragging={dragging || undefined}
          onDragOver={(e) => {
            if (![...e.dataTransfer.items].some((i) => i.kind === 'file')) return
            e.preventDefault()
            setDragging(true)
          }}
          onDragLeave={(e) => !e.currentTarget.contains(e.relatedTarget as Node) && setDragging(false)}
          onDrop={(e) => {
            e.preventDefault()
            setDragging(false)
            void attach(e.dataTransfer.files)
          }}
        >
          <div className="ai-pet-head">
            <strong>{t('aiPet.title')}</strong>
            <span style={{ flex: 1 }} />
            {messages.length > 0 && (
              <button type="button" className="ai-pet-icon" title={t('aiPet.newChat')} aria-label={t('aiPet.newChat')} onClick={() => void resetChat()}>
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
                  <path d="M12 5v14M5 12h14" />
                </svg>
              </button>
            )}
            <button type="button" className="ai-pet-icon" title={t('aiPet.close')} aria-label={t('aiPet.close')} onClick={() => setOpen(false)}>
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
                <path d="M6 6l12 12M18 6L6 18" />
              </svg>
            </button>
          </div>
          {messages.length > 0 && (
            <div className="ai-pet-log" ref={log} aria-live="polite">
              {messages.map((m, i) => (
                <Fragment key={m.id}>
                  <div className={`ai-msg ai-msg-${m.role}`} data-state={m.state}>
                    {m.role === 'user' ? (
                      <>
                        {m.images && (
                          <span className="ai-msg-images">
                            {m.images.map((src, k) => (
                              <img key={k} src={src} alt="" />
                            ))}
                          </span>
                        )}
                        {m.text}
                      </>
                    ) : m.text ? (
                      <Answer text={m.text} />
                    ) : m.state === 'pending' ? (
                      <span className="ai-working">
                        <span className="ai-typing" aria-label={t('aiPet.thinking')}>
                          <i />
                          <i />
                          <i />
                        </span>
                        {activity && <span className="ai-note">{t(`aiPet.tool.${activity}`, { defaultValue: '' })}</span>}
                      </span>
                    ) : null}
                    {m.state === 'stopped' && <span className="ai-note">{t('aiPet.stopped')}</span>}
                    {m.state === 'failed' && <span className="ai-note warn-text">{t('aiPet.failed', { message: m.error ?? '' })}</span>}
                    {m.role === 'assistant' && m.text && !(busy && i === messages.length - 1) && (
                      <button type="button" className="ai-report" aria-pressed={!!m.reported} disabled={m.reported} title={m.reported ? t('aiPet.reported') : t('aiPet.report')} aria-label={m.reported ? t('aiPet.reported') : t('aiPet.report')} onClick={() => void reportAnswer(m.id)}>
                        <svg width="13" height="13" viewBox="0 0 24 24" fill={m.reported ? 'currentColor' : 'none'} stroke="currentColor" strokeWidth="2" strokeLinejoin="round" aria-hidden="true">
                          <path d="M10 15v4a3 3 0 0 0 3 3l4-9V2H5.7a2 2 0 0 0-2 1.7l-1.4 9A2 2 0 0 0 4.3 15zM17 2h3a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-3" />
                        </svg>
                      </button>
                    )}
                  </div>
                  {/* Reply buttons: only under the latest answer, once it is written. */}
                  {i === messages.length - 1 && !busy && m.choices && <Choices choices={m.choices} onPick={(text) => void sendChat(text, language)} onOther={() => input.current?.focus()} />}
                </Fragment>
              ))}
            </div>
          )}
          {images.length > 0 && (
            <div className="ai-attachments">
              {images.map((src, k) => (
                <span key={k} className="ai-attachment">
                  <img src={src} alt="" />
                  <button type="button" aria-label={t('aiPet.removeImage')} title={t('aiPet.removeImage')} onClick={() => setImages((prev) => prev.filter((_, j) => j !== k))}>
                    ×
                  </button>
                </span>
              ))}
            </div>
          )}
          <form
            className="ai-pet-input"
            onSubmit={(e) => {
              e.preventDefault()
              send()
            }}
          >
            <button
              type="button"
              className="ai-pet-icon ai-attach"
              title={t('aiPet.attach')}
              aria-label={t('aiPet.attach')}
              disabled={images.length >= MAX_IMAGES}
              onClick={() => picker.current?.click()}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                <path d="M21.4 11.1l-8.5 8.5a5 5 0 0 1-7.1-7.1l8.5-8.5a3.3 3.3 0 0 1 4.7 4.7l-8.5 8.5a1.7 1.7 0 0 1-2.4-2.4l7.8-7.8" />
              </svg>
            </button>
            <input
              ref={picker}
              type="file"
              accept="image/png,image/jpeg,image/webp,image/gif"
              multiple
              hidden
              onChange={(e) => {
                if (e.target.files) void attach(e.target.files)
                e.target.value = ''
              }}
            />
            <input
              ref={input}
              className="input"
              value={draft}
              placeholder={t(images.length ? 'aiPet.placeholderImage' : 'aiPet.placeholder')}
              maxLength={2000}
              onChange={(e) => setDraft(e.target.value)}
              onPaste={(e) => {
                const files = [...e.clipboardData.files]
                if (files.some((f) => f.type.startsWith('image/'))) {
                  e.preventDefault()
                  void attach(files)
                }
              }}
            />
            {busy ? (
              <button type="button" className="btn" onClick={stopChat}>
                {t('aiPet.stop')}
              </button>
            ) : (
              <button type="submit" className="btn btn-primary" disabled={!draft.trim() && !images.length}>
                {t('aiPet.send')}
              </button>
            )}
          </form>
        </div>
      )}
      <button type="button" className="ai-pet-avatar" data-expression={expression} aria-expanded={open} aria-label={t('aiPet.title')} title={t('aiPet.title')} onClick={() => setOpen(!open)} onPointerEnter={() => setActiveAt(Date.now())}>
        <PixelFace expression={expression} size={52} />
        {unseen && (
          <span className="ai-pet-badge" role="status" aria-label={t('aiPet.newAnswer')}>
            <svg width="14" height="14" viewBox="0 0 8 8" aria-hidden="true">
              {BADGE_DOTS.map(([x, y, lit]) => (
                <circle key={`${x},${y}`} cx={x + 0.5} cy={y + 0.5} r={0.4} className={lit ? 'lit' : undefined} />
              ))}
            </svg>
          </span>
        )}
      </button>
    </div>
  )
}
