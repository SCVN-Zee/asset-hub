export interface GithubCall {
  path: string
  token: string | null
}

interface Reply {
  status: number
  body: unknown
  headers: Record<string, string>
}

/**
 * Stand-in for `api.github.com`: `/user` and `/repos/:owner/:repo`, answered per token. A call that
 * no test configured is recorded in `unexpected` and answered 500, so the suite fails on it.
 */
export class FakeGithub {
  calls: GithubCall[] = []
  unexpected: string[] = []
  private dropped = new Set<string>()
  private users = new Map<string, Reply>()
  private repos = new Map<string, Reply>()

  reset(): void {
    this.calls = []
    this.unexpected = []
    this.dropped.clear()
    this.users.clear()
    this.repos.clear()
  }

  /** Every call with `token` loses its connection, as a dead network would. */
  drop(token: string): void {
    this.dropped.add(token)
  }

  /** `/user` answers `status` for `token` (200 carries `{ login: 'dev' }`). */
  user(token: string, status = 200): void {
    this.users.set(token, { status, body: status === 200 ? { login: 'dev' } : { message: 'x' }, headers: {} })
  }

  /** `/repos/:owner/:repo` answers `status` for `token`. `owner` and `repo` are the lower-case path parts. */
  repo(token: string, owner: string, repo: string, status: number, body: unknown = {}, headers: Record<string, string> = {}): void {
    this.repos.set(`${token}|/repos/${owner}/${repo}`, { status, body, headers })
  }

  async handle(req: Request): Promise<Response> {
    const { pathname } = new URL(req.url)
    const token = /^Bearer\s+(\S+)$/i.exec(req.headers.get('authorization') ?? '')?.[1] ?? null
    this.calls.push({ path: pathname, token })
    if (token && this.dropped.has(token)) throw new Error('connection dropped')
    // GitHub refuses every request without a User-Agent, whatever the token.
    if (!req.headers.get('user-agent')) {
      this.unexpected.push(`${req.method} ${pathname}: no user-agent`)
      return new Response('{"message":"Request forbidden by administrative rules."}', { status: 403, headers: { 'content-type': 'application/json' } })
    }
    const reply = pathname === '/user' ? this.users.get(token ?? '') : this.repos.get(`${token}|${pathname.toLowerCase()}`)
    if (!reply) {
      this.unexpected.push(`${req.method} ${pathname}`)
      return new Response('{"message":"unexpected"}', { status: 500 })
    }
    return new Response(JSON.stringify(reply.body), { status: reply.status, headers: { 'content-type': 'application/json', ...reply.headers } })
  }
}
