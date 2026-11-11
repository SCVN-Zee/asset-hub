import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { withFavorites, type Collection } from '@shared/collections'
import type { RunningUnity } from '@shared/ipc'
import { useCollectionName } from './Collections'
import { UnityButton } from './UnityButton'
import { ChestIcon, ChevronDownIcon, CollectionIcon } from './Icons'

interface Props {
  count: number
  collections: Collection[] | undefined
  busy: boolean
  onChest(): void
  onCollection(id: string): void
  onImportToUnity(editor: RunningUnity): void
  onClear(): void
}

/** Acting on several picked cards at once (Cmd/Ctrl- or Shift-click): chest, a collection, Unity. */
export function MultiBar({ count, collections, busy, onChest, onCollection, onImportToUnity, onClear }: Props) {
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
      <strong>{t('multi.count', { count })}</strong>
      <button type="button" className="btn icon-btn labeled" disabled={busy} title={t('multi.chest')} aria-label={t('multi.chest')} onClick={onChest}>
        <ChestIcon />
        <span>{t('actions.chest')}</span>
      </button>
      <div className="unity-button" ref={box}>
        <button type="button" className="btn icon-btn icon-btn-more labeled" aria-haspopup="menu" aria-expanded={menu} title={t('multi.collection')} aria-label={t('multi.collection')} onClick={() => setMenu(!menu)}>
          <CollectionIcon />
          <span>{t('actions.collection')}</span>
          <ChevronDownIcon size={10} />
        </button>
        {menu && (
          <div className="unity-menu multi-menu" role="menu">
            {withFavorites(collections).map((c) => (
              <button
                key={c.id}
                type="button"
                role="menuitem"
                className="unity-menu-item"
                onClick={() => {
                  setMenu(false)
                  onCollection(c.id)
                }}
              >
                <span>{nameOf(c)}</span>
                <span className="muted small">{c.items.length}</span>
              </button>
            ))}
          </div>
        )}
      </div>
      <UnityButton compact disabled={busy} onImport={onImportToUnity} />
      <span style={{ flex: 1 }} />
      <button type="button" className="link-btn" onClick={onClear} title="Esc">
        {t('multi.clear')}
      </button>
    </div>
  )
}
