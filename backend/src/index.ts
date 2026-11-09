import pkg from '../package.json'
import { assertRepoAccess, authenticate } from './auth'
import type { Env } from './env'
import { getObject, putObject } from './objects'
import { errorJson, json } from './http'
import { getPopular, rebuildPopular } from './popular'
import { postEvents } from './events'
import type { RepoRoute } from './route'
import { getThumbs, listThumbs, putThumbs } from './thumbs'

/** Routes under `/v1/repos/:owner/:repo`. Every one runs behind `authenticate` and `assertRepoAccess`. */
export const repoRoutes: RepoRoute[] = [
  { method: 'POST', pattern: /^\/events$/, handler: postEvents },
  { method: 'GET', pattern: /^\/popular$/, handler: getPopular },
  { method: 'GET', pattern: /^\/cache\/([\w-]+)$/, handler: getObject },
  { method: 'PUT', pattern: /^\/cache\/([\w-]+)$/, handler: putObject },
  { method: 'GET', pattern: /^\/thumbs$/, handler: listThumbs },
  { method: 'POST', pattern: /^\/thumbs\/get$/, handler: getThumbs },
  { method: 'POST', pattern: /^\/thumbs\/put$/, handler: putThumbs }
]

const REPO_PATH = /^\/v1\/repos\/([\w.-]+)\/([\w.-]+)(\/.*)$/

export async function handle(req: Request, env: Env, exec: ExecutionContext): Promise<Response> {
  const { pathname } = new URL(req.url)

  if (pathname === '/v1/health' && req.method === 'GET') return json({ ok: true, version: pkg.version })

  const match = REPO_PATH.exec(pathname)
  if (match) {
    const [, rawOwner, rawRepo, rest] = match
    let params: string[] = []
    const route = repoRoutes.find((r) => {
      if (r.method !== req.method) return false
      const m = r.pattern.exec(rest)
      if (m) params = m.slice(1)
      return m !== null
    })
    if (route) {
      const caller = await authenticate(req, env)
      if (caller instanceof Response) return caller
      const owner = rawOwner.toLowerCase()
      const repo = rawRepo.toLowerCase()
      const access = await assertRepoAccess(caller, owner, repo, env)
      if (access !== true) return access
      return await route.handler({ req, env, exec, caller, owner, repo, slug: `${owner}/${repo}`, params })
    }
  }

  return errorJson('notFound', 404)
}

export default {
  async fetch(req: Request, env: Env, exec: ExecutionContext): Promise<Response> {
    try {
      return await handle(req, env, exec)
    } catch (err) {
      console.error('internal', err instanceof Error ? err.name : 'unknown')
      return errorJson('internal', 500)
    }
  },

  async scheduled(_event: ScheduledController, env: Env, exec: ExecutionContext): Promise<void> {
    exec.waitUntil(rebuildPopular(env))
  }
} satisfies ExportedHandler<Env>
