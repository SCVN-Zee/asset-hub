import type { AssetSource, RepoRef, TreeEntry } from '@shared/types'
import type { IpcError, IpcResult } from '@shared/ipc'

/** Thrown in the renderer for failed IPC calls; `error.code` maps to `errors.<code>`. */
export class RemoteError extends Error {
  constructor(readonly error: IpcError) {
    super(error.message)
  }
}

export function unwrap<T>(result: IpcResult<T>): T {
  if (result.ok) return result.data
  throw new RemoteError(result.error)
}

/** AssetSource backed by the GitHub session in the main process. */
export class RemoteAssetSource implements AssetSource {
  readonly id = 'github'

  async listRepos(): Promise<RepoRef[]> {
    return unwrap(await window.api.source.listRepos())
  }

  async listBranches(repo: string): Promise<string[]> {
    return unwrap(await window.api.source.listBranches(repo))
  }

  async listTree(repo: string, branch: string): Promise<TreeEntry[]> {
    return unwrap(await window.api.source.listTree(repo, branch))
  }
}
