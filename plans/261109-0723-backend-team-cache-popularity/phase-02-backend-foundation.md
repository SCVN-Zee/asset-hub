---
title: "Phase 2: Backend foundation (Worker, bindings, auth)"
status: done
---

# Phase 2: Backend foundation

> Note (phase 3h): the TypeScript sources of this phase were replaced by the Rust Worker in phase 3h and removed.

## Overview

Create a separate Worker project in `backend/`, with its own `package.json`, TypeScript config and tests, so the desktop build and its `tsconfig` (`include: src`) do not change. This phase delivers health and authentication only; data endpoints come in phase 3.

## Requirements

- Free-tier only: Workers Free (10 ms CPU per request), D1, R2 (private bucket), Upstash Redis over REST (`@upstash/redis`).
- The caller sends `Authorization: Bearer <GitHub token>` over TLS. The Worker never stores or logs the token. Cache keys use `sha256(token)`.
- Repo access comes from one call, `GET /repos/{owner}/{repo}`. It returns 404 when the caller cannot read a private repo, and otherwise `permissions.pull` / `permissions.push` for the caller. Reads need `pull`; cache writes need `push`. Public repos are served only if the owner is allowed and `pull` is true, and writes still need `push`.
- Results are cached in Redis for 300 s with no sliding expiry. A denial or error is never cached. Revocation applies to later requests; data already downloaded stays on the Mac.
- GitHub rate limits (429, or 403 with `x-ratelimit-remaining: 0`) are not access denials: return 503 `{ error: 'upstream' }`.
- Only owners in `ALLOWED_OWNERS` are served.
- If Redis is down, auth still works by calling GitHub directly (slower, still correct).

## Tasks

### 2.1 Scaffold the Worker project
- **Goal:** `backend/` builds and tests on its own.
- **Target files:** `backend/package.json`, `backend/wrangler.toml`, `backend/tsconfig.json`, `backend/vitest.config.ts`, `backend/src/index.ts`, `backend/.gitignore` (`.dev.vars`, `.wrangler/`, `node_modules/`).
- **Steps:**
  1. `cd backend && npm init -y`; add devDependencies `wrangler`, `typescript`, `vitest`, `@cloudflare/vitest-pool-workers`, `@cloudflare/workers-types`; dependency `@upstash/redis`. Pin exact versions in `package.json`.
  2. Scripts: `"test": "vitest run"`, `"typecheck": "tsc --noEmit"`, `"dev": "wrangler dev"`, `"deploy": "wrangler deploy"`.
  3. `wrangler.toml`: `name = "asset-hub-api"`, `main = "src/index.ts"`, a `compatibility_date`, `[[d1_databases]] binding = "DB"`, `[[r2_buckets]] binding = "CACHE"`, `[vars] ALLOWED_OWNERS = "supercent-io,SCVN-Dan"`, `[triggers] crons = ["17 3 * * *"]`. Secrets `UPSTASH_REDIS_REST_URL` and `UPSTASH_REDIS_REST_TOKEN` are set with `wrangler secret put` (phase 7), and locally in `.dev.vars` (git-ignored).
  4. `src/index.ts` routes `GET /v1/health` to `{ ok: true, version }` with no auth.
  5. Add `backend/node_modules` to the root `.gitignore` if not covered, and exclude `backend/**` from the root vitest `include` (it is `src/**/*.test.ts` already, so confirm only).
- **Success criteria:** the project installs and the health route returns 200 in tests.
- **Verify:** `cd backend && npm install && npm run typecheck && npm test` exits 0 with at least one passing test for `/v1/health`.

### 2.2 Redis wrapper with its own breaker
- **Goal:** Redis failures never fail a request that can work without Redis.
- **Target:** `backend/src/redis.ts` — `export function redis(env): SafeRedis` with `get`, `set(key, value, ttlSec)`, `zincrby`, `zrevrange`, each returning `undefined` on error or after 300 ms.
- **Steps:**
  1. Wrap `@upstash/redis` `Redis` calls in `Promise.race` with a 300 ms timeout.
  2. On error, log `redis.error` with the command name only (no keys that contain hashes of tokens).
- **Success criteria:** a thrown or slow Redis client returns `undefined`.
- **Verify:** `cd backend && npx vitest run src/redis.test.ts` passes tests "returns undefined on throw" and "returns undefined after 300 ms".

### 2.3 Authentication and repo access
- **Goal:** each `/v1/repos/:owner/:repo/*` request knows the caller and proves repo access before any data is read.
- **Target:** `backend/src/auth.ts` — `authenticate(req, env): Promise<{ login: string; tokenKey: string } | Response>` and `assertRepoAccess(ctx, owner, repo, env): Promise<true | Response>`.
- **Steps:**
  1. Read the bearer token. Missing → 401 `{ error: 'auth' }`.
  2. `tokenKey = hex(sha256(token))`. Look up `auth:<tokenKey>` in Redis; on miss call `GET https://api.github.com/user` with `User-Agent: asset-hub-api`. 401 → return 401. Success → cache `{login}` 300 s.
  3. `owner` not in `ALLOWED_OWNERS` (case-insensitive) → 403.
  4. Lower-case `owner/repo`. Look up `acl:<tokenKey>:<owner>/<repo>`; on miss call `GET /repos/{owner}/{repo}`. 200 with `permissions.pull` → cache `{ pull, push }` for 300 s. 404, 401, or 403 without rate-limit headers → 403 `{ error: 'noAccess' }`, cache nothing. Rate limit or 5xx → 503 `{ error: 'upstream' }`.
  5. Expose `ctx.canPush` to handlers (phase 3 writes need it).
  6. Never pass the token to `console.*`; add a test that captures console output.
- **Success criteria:** revoked tokens fail within 300 s; a token with access to repo A cannot read repo B.
- **Verify:** `cd backend && npx vitest run src/auth.test.ts` passes, with GitHub mocked via `fetchMock` from `cloudflare:test`, these named tests:
  - "rejects missing token" (401),
  - "rejects owner outside allowlist" (403),
  - "denies repo B when only repo A is readable" (403),
  - "revoked token denied after auth cache expiry" (advance time 301 s → 401),
  - "revoked repo access denied after ACL cache expiry" (advance time 301 s → 403),
  - "rate-limited 403 returns 503, not noAccess",
  - "owner and repo are case-insensitive",
  - "push flag follows permissions.push",
  - "works when Redis is down" (Redis wrapper returns undefined → GitHub called → 200),
  - "never logs the token" (console output does not contain the token string).

### 2.4 Router and error shape
- **Goal:** one place maps paths to handlers and errors to JSON.
- **Target:** `backend/src/index.ts` — `fetch(req, env, ctx)` and `scheduled(event, env, ctx)` (empty until phase 3).
- **Steps:**
  1. Route table: `/v1/health`; `/v1/repos/:owner/:repo/...` → `authenticate` → `assertRepoAccess` → handler (handlers added in phase 3).
  2. Responses set no CORS headers; the only client is the desktop app, not a browser.
  3. Unknown path → 404 `{ error: 'notFound' }`. Thrown error → 500 `{ error: 'internal' }` with no stack in the body.
- **Success criteria:** every response is JSON with a stable `error` code on failure.
- **Verify:** `cd backend && npm test` exits 0 and includes "unknown path returns 404 json".

## Success Criteria

- `cd backend && npm run typecheck && npm test` pass.
- No secret or token value appears in any committed file: `git grep -nE "UPSTASH_REDIS_REST_TOKEN\s*=|ghp_|gho_" backend` prints nothing.

## Failure Protocol
If any Verify step does not meet its stated pass condition, STOP this phase.
Do not improvise a fix, retry blindly, or reason around the failure.
Spawn the `kongming` subagent for next-step counsel and pass:
- the phase and task id,
- what you attempted (the steps you ran),
- the exact command and its full output,
- the pass condition it failed to meet.
Apply kongming's guidance, then re-run the Verify step.
If `kongming` cannot be spawned in this environment, STOP and report the same
failure evidence to the user. Never continue by self-reasoning.
