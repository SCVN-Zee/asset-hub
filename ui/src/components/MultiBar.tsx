import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { withFavorites, type Collection } from '@shared/collections'
import type { RunningUnity } from '@shared/ipc'
import { useCollectionName } from './Collections'
import { UnityButton } from './UnityButton'
import { ChevronDownIcon } from './Icons'

interface Props {
  count: number
  collections: Collection[] | undefined
  busy: boolean
  onAddToList(): void
  onCollection(id: string): void
  onImportToUnity(editor: RunningUnity): void
  onClear(): void
}

/** Acting on several picked cards at once (Cmd/Ctrl- or Shift-click): the export list, a collection, Unity. */
export function MultiBar({ count, collections, busy, onAddToList, onCollection, onImportToUnity, onClear }: Props) {
  const { t } = useTranslation()
  const nameOf = useCollectionName()
  const [menu, setMenu] = useState(false)
  const box = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!menu) return
    const close = (e: PointerEvent) => !box.current?.contains(e.target as Node) && setMenu(false)
    window.addEventListener('pointerdown', close)
    return () => window.removeEventListener('pointerdown', close)
  }, [menu])

  return (
    <div className="multi-bar" role="toolbar" aria-label={t('multi.count', { count })}>
      <strong className="multi-count">{t('multi.count', { count })}</strong>
      <button type="button" className="btn" disabled={busy} onClick={onAddToList}>
        {t('multi.exportList')}
      </button>
      <div className="unity-button" ref={box}>
        <button type="button" className="btn" aria-haspopup="menu" aria-expanded={menu} title={t('multi.collection')} onClick={() => setMenu(!menu)}>
          {t('actions.collection')}
          <ChevronDownIcon size={10} />
        </button>
        {menu && (
          <div className="menu-pop multi-menu" role="menu">
            {withFavorites(collections).map((c) => (
              <button
                key={c.id}
                type="button"
                role="menuitem"
                className="menu-pop-item"
                onClick={() => {
                  setMenu(false)
                  onCollection(c.id)
                }}
              >
                <span className="menu-pop-label">{nameOf(c)}</span>
                <span className="num">{c.items.length}</span>
              </button>
            ))}
          </div>
        )}
      </div>
      <UnityButton disabled={busy} onImport={onImportToUnity} />
      <button type="button" className="btn multi-clear" onClick={onClear}>
        {t('multi.clear')}
        <kbd>Esc</kbd>
      </button>
    </div>
  )
}
