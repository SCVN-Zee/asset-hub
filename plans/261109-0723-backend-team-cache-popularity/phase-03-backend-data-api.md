---
title: "Phase 3: Backend data API (events, popularity, cache objects, thumbnails)"
status: done
---

# Phase 3: Backend data API

> Note (phase 3h): the TypeScript sources of this phase were replaced by the Rust Worker in phase 3h and removed.

## Overview

All routes sit under `/v1/repos/:owner/:repo/` and pass `authenticate` + `assertRepoAccess` from phase 2 first. `:owner/:repo` is lower-cased before use, so one repo has one key. Data is per repo, so a caller only sees data of repos they can read on GitHub.

Storage split:
- **D1** (durable): usage events and the thumbnail index.
- **R2** (private bucket `CACHE`): catalog, facts, stats and thumbnail objects, served as stored (no parsing in the Worker, to stay inside 10 ms CPU).
- **Redis** (derived): `pop:<owner>/<repo>` sorted sets, rebuilt from D1 by cron. Ingest never writes to Redis, so there is no race between live increments and the rebuild. When Redis is down, `popular` returns 503 and the app uses its local fallback order. This follows the requirement "without team features" and avoids spending D1 reads.

Cache namespacing: every R2 key includes the schema number from the `X-Asset-Hub-Schema` request header, `repos/<owner>/<repo>/s<schema>/...`, for catalogs, facts, stats **and thumbnails**. The desktop bumps it when a fact or card format changes, so app versions with different formats never read each other's data. Dependency results are not shared in this plan (their answer depends on the branch's GUID index, not on one blob SHA).

Trust model:
- Reading a repo's cache needs read access (`permissions.pull`).
- Writing needs push access (`permissions.push`). Phase 2 gets both from one call.
- Writers are trusted teammates, as with today's index repo. The desktop still validates everything it takes in (phase 5).

Free-tier budgets (Workers Free allows 50 external subrequests per request, such as Upstash REST and GitHub, and 1,000 to Cloudflare services, such as R2 and D1):

| Route | External calls | Cloudflare calls |
|---|---|---|
| any repo route | ≤ 3 (auth cache, ACL cache, GitHub on a miss) | — |
| events | 0 Redis | 1 D1 batch |
| popular | 1 Redis | — |
| thumbs/get | 0 | ≤ 40 R2 |

## Tasks

### 3.1 D1 schema and migration
- **Goal:** tables exist locally and in tests.
- **Target:** `backend/migrations/0001_init.sql`.
- **Steps:**
  1. ```sql
     CREATE TABLE events (id TEXT NOT NULL, repo TEXT NOT NULL, sha TEXT NOT NULL, kind TEXT NOT NULL CHECK (kind IN ('view','send')), ts INTEGER NOT NULL, PRIMARY KEY (repo, id));
     CREATE INDEX events_ts ON events(ts);
     CREATE TABLE thumb_rev (repo TEXT NOT NULL, schema INTEGER NOT NULL, rev INTEGER NOT NULL, PRIMARY KEY (repo, schema));
     CREATE TABLE thumbs (repo TEXT NOT NULL, schema INTEGER NOT NULL, name TEXT NOT NULL, size INTEGER NOT NULL, rev INTEGER NOT NULL, PRIMARY KEY (repo, schema, name));
     CREATE INDEX thumbs_cursor ON thumbs(repo, schema, rev, name);
     ```
  2. Apply migrations in the vitest pool setup (`applyD1Migrations` from `cloudflare:test`).
- **Success criteria:** tests run against the schema.
- **Verify:** `cd backend && npx wrangler d1 migrations apply DB --local` exits 0.

### 3.2 Events: `POST /v1/repos/:o/:r/events`
- **Goal:** batched, de-duplicated, anonymous usage events.
- **Target:** `backend/src/events.ts` — `postEvents`.
- **Steps:**
  1. Body `{ events: Array<{ id: string; sha: string; kind: 'view' | 'send'; ts: number }> }`, 1–100 items, body ≤ 32 KB; else 400. `id` is a UUID; `sha` matches `/^[0-9a-f]{40}$/`; `ts` within the last 7 days and at most 5 min ahead.
  2. `INSERT OR IGNORE` all rows in one `DB.batch`. Store no login.
  3. Return `{ accepted, rejected }`, where `rejected` lists ids that failed validation, so the client drops them instead of retrying.
- **Success criteria:** sending the same batch twice stores rows once; one bad event does not fail the batch.
- **Verify:** `cd backend && npx vitest run src/events.test.ts` passes "dedups by id", "rejects 101 events", "reports bad sha in rejected", "no Redis call on ingest".

### 3.3 Popularity: `GET /v1/repos/:o/:r/popular` and cron rebuild
- **Goal:** ranked SHAs per repo, refreshed every 3 hours.
- **Target:** `backend/src/popular.ts` — `getPopular`, `rebuildPopular(env)`; cron in `wrangler.toml` changed to `["17 */3 * * *"]`.
- **Steps:**
  1. `getPopular`: one `ZRANGE pop:<repo> 0 199 REV WITHSCORES` → `{ items: [{ sha, score }], builtAt }`. Redis returns `undefined` → 503 `{ error: 'redisDown' }`. Empty set → `{ items: [] }`.
  2. `rebuildPopular`: one D1 query over the last 60 days, `SELECT repo, sha, kind, ts FROM events WHERE ts > ?`. In JS, score = Σ weight × 0.5^(ageDays / 14), with weight `send` = 3 and `view` = 1. Per repo, keep the top 200.
  3. For each repo, write with one Upstash pipeline: `DEL pop:<repo>:new`, `ZADD pop:<repo>:new <score sha ...>`, `RENAME pop:<repo>:new pop:<repo>`, `EXPIRE pop:<repo> 604800`. One HTTP request; about 4 commands per repo.
  4. Delete events older than 90 days.
  5. Budget check in code comments and in a test: with 51 repos every 3 hours, Redis is ~4 × 51 × 8 × 30 ≈ 49K commands/month, and D1 reads are the 60-day event count × 8 per day.
- **Success criteria:** ranking follows weights and decay; Redis down gives 503, not a D1 scan.
- **Verify:** `cd backend && npx vitest run src/popular.test.ts` passes "send outweighs view", "older events decay", "keeps top 200", "503 when Redis is down", "rebuild uses one pipeline per repo".

### 3.4 Cache objects: catalog, facts, stats
- **Goal:** one gzip JSON object per kind per repo, with safe concurrent writes.
- **Target:** `backend/src/objects.ts` — `getObject`, `putObject`. Routes `GET|PUT /v1/repos/:o/:r/cache/:name`.
- **Steps:**
  1. `name` must be in an explicit allowlist: `catalog`, `stats`, and `facts-<kind>` for each kind in `SHARED_FACT_KINDS` (`src/core/github/source.ts`: `metaguids`, `prefabmodels`, `prefabfx3`, `spritemeta`, `tmpfont`; copy the exact strings into `backend/src/kinds.ts` and add a desktop test that the two lists match). Anything else → 404.
  2. Key `repos/<o>/<r>/s<schema>/<name>.json.gz`; missing or non-integer `X-Asset-Hub-Schema` → 400.
  3. GET: honor `If-None-Match` → 304; else stream the R2 body with `ETag`, `Content-Type: application/gzip`.
  4. PUT: needs `permissions.push` (403 `{ error: 'readOnly' }` otherwise); body ≤ 20 MB and starts with gzip magic `1f 8b`, else 413/400. Require `If-Match: <etag>` or `If-None-Match: *`; use R2 `put` with `onlyIf`; precondition failed → 412 (the client merges again and retries).
- **Success criteria:** two writers with the same old ETag: one gets 200, the other 412; a reader without push gets 403 on PUT.
- **Verify:** `cd backend && npx vitest run src/objects.test.ts` passes "304 on matching etag", "412 on stale etag", "rejects non-gzip", "accepts facts-prefabfx3", "rejects unknown name", "schema namespaces keys", "PUT needs push", "repo B object not readable with repo A access".

### 3.5 Thumbnails: list, batch get, batch put
- **Goal:** a new Mac fetches a repo's thumbnails in a few hundred requests, within subrequest and burst limits, and never misses one at a page boundary.
- **Target:** `backend/src/thumbs.ts`, `backend/src/pack.ts`. Routes:
  - `GET /v1/repos/:o/:r/thumbs?after=<rev>:<name>&limit=1000` → `{ items: [{ name, size, rev }], next?: '<rev>:<name>' }`, ordered by `(rev, name)`. `rev` is a per-(repo, schema) counter bumped in the same D1 batch as each put, so a replacement is always listed again, even within one millisecond.
  - `POST /v1/repos/:o/:r/thumbs/get` body `{ names: string[] }` (≤ 40) → binary pack; missing names are left out.
  - `POST /v1/repos/:o/:r/thumbs/put` binary pack (≤ 20 entries, ≤ 4 MB); needs `permissions.push`.
- **Steps:**
  1. `name` matches `/^[0-9a-f]{40}(\.fx)?$/` (names of `ThumbnailStore` in `src/main/caches.ts`).
  2. Pack format, both directions: repeated `[u16 nameLength][name utf8][u32 byteLength][bytes]`. Encoder/decoder in `backend/src/pack.ts`; the same code in `src/shared/thumbPack.ts` for the desktop (it cannot import from `backend/`), with identical test vectors in both test files.
  3. R2 key `repos/<o>/<r>/s<schema>/thumbs/<name>`. Put writes R2, then bumps `thumb_rev` and upserts the D1 rows with that `rev` in one batch. Responses stream one entry at a time (no full-pack buffer); one thumbnail is at most 1 MB.
- **Success criteria:** round trip of 40 thumbnails returns identical bytes; paging with many rows sharing one `updated_at` returns each name once.
- **Verify:** `cd backend && npx vitest run src/thumbs.test.ts src/pack.test.ts` passes "round trip 40 thumbs", "rejects 41 names", "rejects bad name", "cursor handles equal timestamps", "put needs push", "replaced thumb listed again".

## Success Criteria

- `cd backend && npm run typecheck && npm test` pass.
- No repo route returns data without a passing `assertRepoAccess`: test "every repo route calls assertRepoAccess" iterates the route table with a denied token and expects 403.

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
