import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { useTranslation } from 'react-i18next'
import type { RunningUnity } from '@shared/ipc'
import { ChevronDownIcon, UnityIcon } from './Icons'
import { useFloatingMenu, useMenuDismiss } from './InspectorParts'

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

interface ControlProps {
  editors: readonly RunningUnity[]
  disabled?: boolean
  onImport(editor: RunningUnity): void
  /** Icon and a short word, for a row with little room. */
  compact?: boolean
  /** Joined to its arrow as one control that fills the room it is given (the inspector's action row). */
  split?: boolean
  /** Where the list opens when there is room both ways. */
  prefer?: 'above' | 'below'
  /** The element that says why the button is off (shown always, not only on hover). */
  describedBy?: string
}

/**
 * "Import into Unity": the open Unity editors are listed (several projects can be
 * open); the button goes to the one used last, the arrow picks another. Without
 * an open editor the button is off; the caller shows why in text next to it.
 */
export function UnityControl({ editors, disabled, onImport, compact, split, prefer = 'above', describedBy }: ControlProps) {
  const { t } = useTranslation()
  const [menu, setMenu] = useState(false)
  const box = useRef<HTMLDivElement>(null)
  const { list, pos } = useFloatingMenu(menu, box, 'start', prefer)
  const close = () => setMenu(false)
  useMenuDismiss(menu, close, box, list)
  // A list left open for an editor that has since closed would point at nothing.
  useEffect(() => {
    if (!editors.length) setMenu(false)
  }, [editors.length])

  const label = compact ? (
    <>
      <UnityIcon />
      <span>{t('unity.short')}</span>
    </>
  ) : (
    <>
      {split && <UnityIcon />}
      {t('unity.import')}
    </>
  )
  const cls = compact ? 'btn icon-btn labeled' : 'btn'
  const wrap = `unity-button${split ? ' split' : ''}`

  if (!editors.length)
    return (
      <div className={wrap}>
        <button type="button" className={cls} disabled aria-describedby={describedBy} title={t('unity.noneOpen')} aria-label={t('unity.import')}>
          {label}
        </button>
      </div>
    )

  const preferred = preferredEditor(editors)
  const choose = (editor: RunningUnity) => {
    rememberEditor(editor)
    setMenu(false)
    onImport(editor)
  }

  return (
    <div className={wrap} ref={box}>
      <button
        type="button"
        className={cls}
        disabled={disabled}
        aria-describedby={describedBy}
        title={`${t('unity.importInto', { project: preferred.project })} · ${preferred.projectPath}`}
        aria-label={t('unity.importInto', { project: preferred.project })}
        onClick={() => choose(preferred)}
      >
        {compact || split ? label : t('unity.importInto', { project: preferred.project })}
      </button>
      {editors.length > 1 && (
        <button type="button" className="btn unity-more" disabled={disabled} aria-haspopup="menu" aria-expanded={menu} aria-label={t('unity.pick')} title={t('unity.pick')} onClick={() => setMenu(!menu)}>
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

/** `UnityControl` for the editors open now. */
export function UnityButton(props: Omit<ControlProps, 'editors'>) {
  return <UnityControl editors={useUnityEditors()} {...props} />
}
