import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { SideGroup } from './Section'
import { ClockIcon, CollectionIcon, ShippedIcon, StarIcon } from './Icons'
import { MenuItem } from './InspectorParts'
import type { StatsSummary } from '@shared/types'
import type { HandlerRegistry } from '../handlers'
import { FAVORITES, inCollection, RECENT_EXPORTED, RECENT_VIEWED, type Collection, type CollectionItem } from '@shared/collections'
import { AssetCard } from './AssetCard'

/** Favourites goes by its translated name; the others by the name they were given. */
export function useCollectionName() {
  const { t } = useTranslation()
  return (c: Collection) =>
    c.id === FAVORITES ? t('collections.favorites') : c.id === RECENT_VIEWED ? t('recent.viewed') : c.id === RECENT_EXPORTED ? t('recent.exported') : c.name || t('collections.untitled')
}

const newId = () => `c${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`

/** A name typed in place: Enter keeps it, Escape (or leaving it empty) drops it. */
function NameInput({ initial = '', placeholder, onDone }: { initial?: string; placeholder: string; onDone(name: string | undefined): void }) {
  const [name, setName] = useState(initial)
  return (
    <input
      className="input collection-name-input"
      autoFocus
      value={name}
      placeholder={placeholder}
      onChange={(e) => setName(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') onDone(name.trim() || undefined)
        if (e.key === 'Escape') onDone(undefined)
      }}
      onBlur={() => onDone(name.trim() || undefined)}
    />
  )
}

/** Sidebar group "Yours": Favourites, then the named collections, a way to make one, and the recent lists; each opens in the grid. */
export function CollectionsNav({
  collections,
  recent,
  open,
  onOpen,
  onCreate
}: {
  collections: Collection[]
  /** Recently viewed and exported assets, opened in the grid like a collection. */
  recent: Collection[]
  open?: string
  onOpen(id: string | undefined): void
  onCreate(id: string, name: string): void
}) {
  const { t } = useTranslation()
  const nameOf = useCollectionName()
  const [adding, setAdding] = useState(false)
  return (
    <SideGroup id="collections" title={t('sidebar.yours')} className="collections-nav" folded={collections.length}>
      <div className="tabs" role="group" aria-label={t('sidebar.yours')}>
        {collections.map((c) => (
          <button key={c.id} type="button" className="tab" aria-pressed={open === c.id} onClick={() => onOpen(open === c.id ? undefined : c.id)}>
            <span className="nav-icon">{c.id === FAVORITES ? <StarIcon size={14} /> : <CollectionIcon size={14} />}</span>
            <span className="collection-label">{nameOf(c)}</span>
            <span className="num">{c.items.length}</span>
          </button>
        ))}
        {adding ? (
          <NameInput
            placeholder={t('collections.namePlaceholder')}
            onDone={(name) => {
              setAdding(false)
              if (!name) return
              const id = newId()
              onCreate(id, name)
              onOpen(id)
            }}
          />
        ) : (
          <button type="button" className="collection-new" onClick={() => setAdding(true)}>
            <span className="collection-new-plus" aria-hidden="true">
              +
            </span>
            {t('collections.new')}
          </button>
        )}
        {recent.map((c) => (
          <button key={c.id} type="button" className="tab" aria-pressed={open === c.id} disabled={!c.items.length} onClick={() => onOpen(open === c.id ? undefined : c.id)}>
            <span className="nav-icon">{c.id === RECENT_VIEWED ? <ClockIcon size={14} /> : <ShippedIcon size={14} />}</span>
            <span className="collection-label">{nameOf(c)}</span>
            <span className="num">{c.items.length}</span>
          </button>
        ))}
      </div>
    </SideGroup>
  )
}

interface ViewProps {
  collection: Collection
  registry: HandlerRegistry
  thumbs: Record<string, string>
  stats: Record<string, StatsSummary>
  /** What the details panel shows now, to mark its card. */
  current: { repo: string; branch: string; path?: string }
  onOpenItem(item: CollectionItem): void
  onRename(name: string): void
  onDelete(): void
  onClose(): void
}

/** A collection in the grid: its assets from any repo or branch; a card opens the asset where it was added from. */
export function CollectionView({ collection, registry, thumbs, stats, current, onOpenItem, onRename, onDelete, onClose }: ViewProps) {
  const { t } = useTranslation()
  const nameOf = useCollectionName()
  const [renaming, setRenaming] = useState(false)
  const [confirm, setConfirm] = useState(false)
  useEffect(() => {
    setRenaming(false)
    setConfirm(false)
  }, [collection.id])
  // Favourites and the recent lists are the app's: they cannot be renamed or deleted.
  const own = collection.id !== FAVORITES && !collection.id.startsWith('recent:')
  return (
    <>
      <div className="content-head">
        {renaming ? (
          <NameInput
            initial={collection.name}
            placeholder={t('collections.namePlaceholder')}
            onDone={(name) => {
              setRenaming(false)
              if (name) onRename(name)
            }}
          />
        ) : (
          <h1>{nameOf(collection)}</h1>
        )}
        <span className="collection-actions small">
          <span className="muted">{t('collections.count', { count: collection.items.length })}</span>
          {own && !renaming && (
            <button type="button" className="inline-link" onClick={() => setRenaming(true)}>
              {t('collections.rename')}
            </button>
          )}
          {own &&
            (confirm ? (
              <>
                <button type="button" className="inline-link danger-text" onClick={onDelete}>
                  {t('collections.deleteConfirm', { name: nameOf(collection) })}
                </button>
                <button type="button" className="inline-link" onClick={() => setConfirm(false)}>
                  {t('collections.keep')}
                </button>
              </>
            ) : (
              <button type="button" className="inline-link" onClick={() => setConfirm(true)}>
                {t('collections.delete')}
              </button>
            ))}
          <button type="button" className="inline-link" onClick={onClose}>
            {t('collections.close')}
          </button>
        </span>
      </div>
      {collection.items.length === 0 ? (
        <p className="empty">{t('collections.empty')}</p>
      ) : (
        <div className="grid">
          {collection.items.map((item) => {
            const elsewhere = item.repo !== current.repo || item.branch !== current.branch
            return (
              <AssetCard
                key={`${item.repo}\n${item.path}`}
                asset={{ id: item.path, type: item.type, path: item.path, name: item.name, sha: item.sha, size: item.size, lfs: false }}
                handler={registry.get(item.type)}
                thumb={thumbs[item.sha]}
                stats={stats[item.sha]}
                selected={!elsewhere && current.path === item.path}
                onSelect={() => onOpenItem(item)}
                note={elsewhere ? `${item.repo.split('/').pop()} · ${item.branch}` : undefined}
              />
            )
          })}
        </div>
      )}
    </>
  )
}

/** The star: in or out of Favourites. */
export function FavoriteButton({ collections, item, onToggle }: { collections: Collection[]; item: CollectionItem; onToggle(id: string): void }) {
  const { t } = useTranslation()
  const starred = inCollection(collections.find((c) => c.id === FAVORITES), item)
  const label = starred ? t('collections.unstar') : t('collections.star')
  return (
    <button type="button" className="btn icon-btn star-btn" aria-pressed={starred} title={label} aria-label={label} onClick={() => onToggle(FAVORITES)}>
      <StarIcon filled={starred} />
    </button>
  )
}

/**
 * Rows for a menu: every collection, ticked when the asset is in it (a tick keeps the menu open, to tick several),
 * and a way to make one holding the asset. `onCreate` gets the new id and its name.
 */
export function CollectionMenuItems({ collections, item, onToggle, onCreate }: { collections: Collection[]; item: CollectionItem; onToggle(id: string): void; onCreate(id: string, name: string): void }) {
  const { t } = useTranslation()
  const nameOf = useCollectionName()
  const [adding, setAdding] = useState(false)
  const named = collections.filter((c) => c.id !== FAVORITES)
  return (
    <>
      {named.map((c) => (
        <MenuItem key={c.id} label={nameOf(c)} checked={inCollection(c, item)} keepOpen onSelect={() => onToggle(c.id)} />
      ))}
      {named.length === 0 && !adding && <p className="menu-empty muted small">{t('collections.none')}</p>}
      {adding ? (
        <div className="menu-field">
          <NameInput
            placeholder={t('collections.namePlaceholder')}
            onDone={(name) => {
              setAdding(false)
              if (!name) return
              // Made with the asset already in it (one settings update, not two racing).
              onCreate(newId(), name)
            }}
          />
        </div>
      ) : (
        <MenuItem icon={<CollectionIcon size={14} />} label={t('collections.new')} keepOpen onSelect={() => setAdding(true)} />
      )}
    </>
  )
}
