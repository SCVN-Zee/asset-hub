/** Links that open an asset in the app: assethub://open?repo=owner/name&branch=main&path=Assets/… */
export interface AssetLink {
  repo: string
  branch: string
  /** Repo path of the file. */
  path: string
}

export const LINK_SCHEME = 'assethub'

export function assetLink(link: AssetLink): string {
  const q = new URLSearchParams({ repo: link.repo, branch: link.branch, path: link.path })
  return `${LINK_SCHEME}://open?${q.toString()}`
}

/** The asset a link points at; undefined for anything else (or a link that could reach outside the repo). */
export function parseAssetLink(text: string): AssetLink | undefined {
  const trimmed = text.trim()
  if (!trimmed.toLowerCase().startsWith(`${LINK_SCHEME}://`)) return undefined
  let url: URL
  try {
    url = new URL(trimmed)
  } catch {
    return undefined
  }
  if (url.hostname !== 'open') return undefined
  const repo = url.searchParams.get('repo') ?? ''
  const branch = url.searchParams.get('branch') ?? ''
  const path = url.searchParams.get('path') ?? ''
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo) || !branch || !path) return undefined
  if (path.startsWith('/') || path.split('/').includes('..')) return undefined
  return { repo, branch, path }
}
