import { describe, expect, it } from 'vitest'
import { assetLink, parseAssetLink } from './deeplink'

describe('asset links', () => {
  it('round-trips repo, branch and a path with spaces and Unicode', () => {
    const link = { repo: 'supercent-io/hb_creative_xp_hero', branch: 'vn/asmr-fish', path: 'unity_project/Assets/FX/6(C)_ Crocodile Green.fbx' }
    const url = assetLink(link)
    expect(url.startsWith('assethub://open?')).toBe(true)
    expect(parseAssetLink(url)).toEqual(link)
    expect(parseAssetLink(`  ${url}\n`)).toEqual(link)
  })

  it('ignores other text and unsafe links', () => {
    expect(parseAssetLink('tree')).toBeUndefined()
    expect(parseAssetLink('https://github.com/a/b')).toBeUndefined()
    expect(parseAssetLink('assethub://open?repo=a/b&branch=main')).toBeUndefined()
    expect(parseAssetLink('assethub://open?repo=a/b&branch=main&path=../../etc/passwd')).toBeUndefined()
    expect(parseAssetLink('assethub://open?repo=not a repo&branch=main&path=A.fbx')).toBeUndefined()
    expect(parseAssetLink('assethub://delete?repo=a/b&branch=main&path=A.fbx')).toBeUndefined()
  })
})
