import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { SideGroup } from './Section'
import { ChevronDownIcon, ClockIcon, CollectionIcon, ShippedIcon, StarIcon } from './Icons'
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

/** Sidebar: recently viewed and exported assets, opened in the grid like a collection. */
export function RecentNav({ lists, open, onOpen }: { lists: Collection[]; open?: string; onOpen(id: string | undefined): void }) {
  const { t } = useTranslation()
  const nameOf = useCollectionName()
  if (!lists.some((l) => l.items.length)) return null
  return (
    <SideGroup id="recent" title={t('recent.title')} className="collections-nav" folded={lists.reduce((n, l) => n + l.items.length, 0)}>
      <div className="tabs" role="group" aria-label={t('recent.title')}>
        {lists.map((c) => (
          <button key={c.id} type="button" className="tab" aria-pressed={open === c.id} disabled={!c.items.length} onClick={() => onOpen(open === c.id ? undefined : c.id)}>
            <span className="nav-icon">{c.id === RECENT_VIEWED ? <ClockIcon size={14} /> : <ShippedIcon size={14} />}</span>
            <span className="collection-label">{nameOf(c)}</span> <span className="muted">{c.items.length}</span>
          </button>
        ))}
      </div>
    </SideGroup>
  )
}

/** Sidebar list: Favourites, then the named collections; one opens in the grid. */
export function CollectionsNav({ collections, open, onOpen, onCreate }: { collections: Collection[]; open?: string; onOpen(id: string | undefined): void; onCreate(id: string, name: string): void }) {
  const { t } = useTranslation()
  const nameOf = useCollectionName()
  const [adding, setAdding] = useState(false)
  return (
    <SideGroup id="collections" title={t('collections.title')} className="collections-nav" folded={collections.length}>
      <div className="tabs" role="group" aria-label={t('collections.title')}>
        {collections.map((c) => (
          <button key={c.id} type="button" className="tab" aria-pressed={open === c.id} onClick={() => onOpen(open === c.id ? undefined : c.id)}>
            <span className="nav-icon">{c.id === FAVORITES ? <StarIcon size={14} /> : <CollectionIcon size={14} />}</span>
            <span className="collection-label">{nameOf(c)}</span> <span className="muted">{c.items.length}</span>
          </button>
        ))}
      </div>
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
          <span className="collection-new-plus" aria-hidden="true">+</span>
          {t('collections.new')}
        </button>
      )}
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

/** Details panel: the star (Favourites) and a menu ticking the asset in or out of each collection; `onCreate` makes one holding the asset. */
export function CollectionButtons({ collections, item, onToggle, onCreate }: { collections: Collection[]; item: CollectionItem; onToggle(id: string): void; onCreate(id: string, name: string): void }) {
  const { t } = useTranslation()
  const nameOf = useCollectionName()
  const [open, setOpen] = useState(false)
  const [adding, setAdding] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const starred = inCollection(collections.find((c) => c.id === FAVORITES), item)
  const named = collections.filter((c) => c.id !== FAVORITES)
  const inNamed = named.filter((c) => inCollection(c, item)).length

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => !root.current?.contains(e.target as Node) && setOpen(false)
    document.addEventListener('mousedown', close)
    return () => document.removeEventListener('mousedown', close)
  }, [open])

  return (
    <div className="collection-buttons" ref={root}>
      <button type="button" className="btn icon-btn star-btn" aria-pressed={starred} title={starred ? t('collections.unstar') : t('collections.star')} aria-label={starred ? t('collections.unstar') : t('collections.star')} onClick={() => onToggle(FAVORITES)}>
        <StarIcon filled={starred} />
      </button>
      <button type="button" className="btn icon-btn icon-btn-more labeled" aria-expanded={open} aria-haspopup="true" title={t('collections.addTo')} aria-label={t('collections.addTo')} onClick={() => setOpen((o) => !o)}>
        <CollectionIcon />
        <span>{t('actions.collection')}</span>
        {inNamed > 0 && <span className="icon-count">{inNamed}</span>}
        <ChevronDownIcon size={10} />
      </button>
      {open && (
        <div className="collection-pop" role="menu">
          {named.length === 0 && !adding && <p className="muted small">{t('collections.none')}</p>}
          {named.map((c) => (
            <label key={c.id} className="check" role="menuitemcheckbox" aria-checked={inCollection(c, item)}>
              <input type="checkbox" checked={inCollection(c, item)} onChange={() => onToggle(c.id)} />
              {nameOf(c)}
            </label>
          ))}
          {adding ? (
            <NameInput
              placeholder={t('collections.namePlaceholder')}
              onDone={(name) => {
                setAdding(false)
                if (!name) return
                // Made with the asset already in it (one settings update, not two racing).
                onCreate(newId(), name)
              }}
            />
          ) : (
            <button type="button" className="inline-link small" onClick={() => setAdding(true)}>
              + {t('collections.new')}
            </button>
          )}
        </div>
      )}
    </div>
  )
}
