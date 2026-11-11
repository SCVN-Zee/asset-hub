import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { useTranslation } from 'react-i18next'
import type { RunningUnity } from '@shared/ipc'
import { ChevronDownIcon, UnityIcon } from './Icons'

const LAST_KEY = 'assethub.lastUnityProject'
const lastProject = () => {
  try {
    return localStorage.getItem(LAST_KEY) ?? ''
  } catch {
    return ''
  }
}

/** The Unity editors open now, refreshed every 15 s and when the app comes back to the front. */
export function useUnityEditors(): RunningUnity[] {
  const [editors, setEditors] = useState<RunningUnity[]>([])
  useEffect(() => {
    const refresh = () => void window.api.exporter.unityEditors().then(setEditors)
    refresh()
    window.addEventListener('focus', refresh)
    const timer = window.setInterval(refresh, 15_000)
    return () => {
      window.removeEventListener('focus', refresh)
      window.clearInterval(timer)
    }
  }, [])
  return editors
}

/** The editor imports go to by default: the project used last, else the first open one. */
export const preferredEditor = (editors: readonly RunningUnity[]) => editors.find((e) => e.projectPath === lastProject()) ?? editors[0]

/** Remembers the project imported into, for next time. */
export function rememberEditor(editor: RunningUnity): void {
  try {
    localStorage.setItem(LAST_KEY, editor.projectPath)
  } catch {
    // not remembered
  }
}

/**
 * "Import into Unity": the open Unity editors are listed (several projects can be
 * open); the button goes to the one used last, the arrow picks another. Without
 * an open editor it waits for one, refreshed when the app comes back to the front.
 */
export function UnityButton({ disabled, onImport, compact }: { disabled?: boolean; onImport(editor: RunningUnity): void; compact?: boolean }) {
  const { t } = useTranslation()
  const editors = useUnityEditors()
  const [menu, setMenu] = useState(false)
  const box = useRef<HTMLDivElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)

  useEffect(() => {
    if (!menu) return
    const close = (e: PointerEvent) => !box.current?.contains(e.target as Node) && !list.current?.contains(e.target as Node) && setMenu(false)
    const shut = () => setMenu(false)
    window.addEventListener('pointerdown', close)
    window.addEventListener('resize', shut)
    window.addEventListener('blur', shut)
    return () => {
      window.removeEventListener('pointerdown', close)
      window.removeEventListener('resize', shut)
      window.removeEventListener('blur', shut)
    }
  }, [menu])
  // The list sits on the window, not in the button's panel: kept inside the window's
  // edges (above the button, or below when there is no room), never cut off.
  useLayoutEffect(() => {
    if (!menu) return setPos(null)
    const anchor = box.current?.getBoundingClientRect()
    const el = list.current
    if (!anchor || !el) return
    const gap = 6
    const edge = 8
    const left = Math.max(edge, Math.min(anchor.left, window.innerWidth - el.offsetWidth - edge))
    const above = anchor.top - el.offsetHeight - gap
    const top = above >= edge ? above : Math.min(anchor.bottom + gap, window.innerHeight - el.offsetHeight - edge)
    setPos({ left, top: Math.max(edge, top) })
  }, [menu])

  if (!editors.length)
    return (
      <button type="button" className={compact ? 'btn icon-btn labeled' : 'btn'} disabled title={t('unity.noneOpen')} aria-label={t('unity.import')}>
        {compact ? (
          <>
            <UnityIcon />
            <span>Unity</span>
          </>
        ) : (
          t('unity.import')
        )}
      </button>
    )

  const preferred = preferredEditor(editors)
  const choose = (editor: RunningUnity) => {
    rememberEditor(editor)
    setMenu(false)
    onImport(editor)
  }

  return (
    <div className="unity-button" ref={box}>
      <button type="button" className={compact ? 'btn icon-btn labeled' : 'btn'} disabled={disabled} title={`${t('unity.importInto', { project: preferred.project })} · ${preferred.projectPath}`} aria-label={t('unity.importInto', { project: preferred.project })} onClick={() => choose(preferred)}>
        {compact ? (
          <>
            <UnityIcon />
            <span>Unity</span>
          </>
        ) : (
          t('unity.importInto', { project: preferred.project })
        )}
      </button>
      {editors.length > 1 && (
        <button type="button" className="btn unity-more" disabled={disabled} aria-haspopup="menu" aria-expanded={menu} aria-label={t('unity.pick')} onClick={() => setMenu(!menu)}>
          <ChevronDownIcon size={10} />
        </button>
      )}
      {menu &&
        createPortal(
          <div className="unity-menu" role="menu" ref={list} style={pos ? { left: pos.left, top: pos.top } : { visibility: 'hidden' }}>
          {editors.map((e) => (
            <button key={e.pid} type="button" role="menuitem" className="unity-menu-item" aria-current={e.pid === preferred.pid || undefined} title={e.projectPath} onClick={() => choose(e)}>
              <span>
                {e.project}
                {e.product && <span className="muted small"> · {e.product}</span>}
              </span>
              {e.version && <span className="muted small">{e.version}</span>}
            </button>
          ))}
          </div>,
          document.body
        )}
    </div>
  )
}
