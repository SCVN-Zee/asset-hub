import { describe, expect, it } from 'vitest'
import { addCollection, deleteCollection, FAVORITES, inCollection, renameCollection, sanitizeCollections, toggleItem, withFavorites, type CollectionItem } from './collections'

const item = (path: string, branch = 'main'): CollectionItem => ({ repo: 'o/r', branch, path, sha: 'a'.repeat(40), name: path.split('/').pop()!, type: 'fbx' })

describe('collections', () => {
  it('always has Favourites first', () => {
    expect(withFavorites(undefined).map((c) => c.id)).toEqual([FAVORITES])
    const list = addCollection(undefined, 'c1', '  Trees ')
    expect(list.map((c) => [c.id, c.name])).toEqual([[FAVORITES, ''], ['c1', 'Trees']])
  })

  it('toggles an asset in and out, by repo and path whichever branch', () => {
    let list = toggleItem(undefined, FAVORITES, item('Assets/Tree.fbx'))
    expect(inCollection(list[0], item('Assets/Tree.fbx', 'dev'))).toBe(true)
    list = toggleItem(list, FAVORITES, item('Assets/Rock.fbx'))
    expect(list[0].items.map((i) => i.name)).toEqual(['Rock.fbx', 'Tree.fbx'])
    list = toggleItem(list, FAVORITES, item('Assets/Tree.fbx', 'dev'))
    expect(list[0].items.map((i) => i.name)).toEqual(['Rock.fbx'])
  })

  it('renames and deletes collections, never Favourites', () => {
    let list = addCollection(undefined, 'c1', 'Trees')
    list = renameCollection(list, 'c1', 'Forest')
    list = renameCollection(list, FAVORITES, 'Nope')
    expect(list.map((c) => c.name)).toEqual(['', 'Forest'])
    expect(deleteCollection(deleteCollection(list, 'c1'), FAVORITES).map((c) => c.id)).toEqual([FAVORITES])
  })

  it('drops malformed stored data', () => {
    expect(sanitizeCollections('x')).toBeUndefined()
    expect(sanitizeCollections([{ id: 'a', name: 'A', items: [item('p'), { repo: 'o/r' }] }, { id: 'a', name: 'dup', items: [] }, null])).toEqual([{ id: 'a', name: 'A', items: [item('p')] }])
  })
})
