import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { FolderNode } from '@shared/types'
import { ChevronRightIcon } from './Icons'

interface Props {
  root: FolderNode
  selected: string
  onSelect(path: string): void
  /** Right-click on a folder. */
  onMenu?(path: string, e: React.MouseEvent): void
}

/** Folders that start expanded: the first two levels. */
function initialOpen(root: FolderNode): Set<string> {
  const open = new Set<string>()
  for (const a of root.children) {
    open.add(a.path)
    for (const b of a.children) open.add(b.path)
  }
  return open
}

export function FolderTree({ root, selected, onSelect, onMenu }: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(() => initialOpen(root))

  const toggle = (path: string) =>
    setOpen((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })

  const renderNode = (node: FolderNode, depth: number) => {
    const isOpen = open.has(node.path)
    const hasChildren = node.children.length > 0
    return (
      <li key={node.path}>
        <button
          type="button"
          className="tree-item"
          style={{ paddingLeft: 8 + depth * 14 }}
          aria-current={selected === node.path}
          aria-expanded={hasChildren ? isOpen : undefined}
          onContextMenu={onMenu && ((e) => onMenu(node.path, e))}
          onClick={() => {
            onSelect(node.path)
            if (hasChildren && (!isOpen || selected === node.path)) toggle(node.path)
          }}
        >
          <span className="tree-caret" data-open={isOpen}>
            {hasChildren && <ChevronRightIcon />}
          </span>
          <span className="tree-name">{node.name}</span>
          <span className="count">{node.assetCount}</span>
        </button>
        {hasChildren && isOpen && <ul className="tree">{node.children.map((c) => renderNode(c, depth + 1))}</ul>}
      </li>
    )
  }

  return (
    <ul className="tree">
      <li>
        <button type="button" className="tree-item" aria-current={selected === ''} onClick={() => onSelect('')}>
          <span className="tree-caret" />
          <span className="tree-name">{t('browse.allFolders')}</span>
          <span className="count">{root.assetCount}</span>
        </button>
      </li>
      {root.children.map((c) => renderNode(c, 0))}
    </ul>
  )
}
