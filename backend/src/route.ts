import type { AuthContext } from './auth'
import type { Env } from './env'

/** What a repo handler gets once the caller has proven read access to `owner/repo`. */
export interface RepoRequest {
  req: Request
  env: Env
  exec: ExecutionContext
  caller: AuthContext
  owner: string
  repo: string
  /** `owner/repo`, lower-case. */
  slug: string
  /** Capture groups of the route pattern. */
  params: string[]
}

export type RepoHandler = (r: RepoRequest) => Promise<Response>

export interface RepoRoute {
  method: string
  /** Matched against the path after `/v1/repos/:owner/:repo`. */
  pattern: RegExp
  handler: RepoHandler
}
