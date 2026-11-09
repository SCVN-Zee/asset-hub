---
title: "Phase 4: Desktop backend client (fail-open)"
status: todo
---

# Phase 4: Desktop backend client

## Overview

One module in the main process talks to the backend. It must never slow local work: every call has a deadline, failures open a breaker, and callers always have a local fallback. Team features turn off quietly while the backend is unreachable and come back on their own.

## Requirements

- Backend URL from `package.json` key `backendUrl` (beside `releaseRepo`, `feedbackRepo`). Empty or missing → backend disabled; the app behaves exactly like 1.0.5.
- Uses the signed-in GitHub token; never writes it to disk or logs. Sends it only to an `https:` `backendUrl`, with `redirect: 'error'`, so the token never follows a redirect.
- Sign-out or account change: abort every in-flight call, clear the in-memory popularity cache, and drop the queued events (they were made under the old account).
- Operation budget: 1.5 s for interactive reads (popularity), 30 s for background sync requests. A call past its budget is abandoned with `AbortController` **and** settled by its own timer (`Promise.race`), so even a fetch that ignores the abort returns `undefined` on time. The budget covers the whole call: request, body read and parsing.
- Breaker per service area (`api`): open after 3 failures in a row (network error, timeout, 5xx, 429, corrupt body); stay open 60 s ± 20 % jitter; one probe call when half-open. 401/403 are not breaker failures: they mean no access to that repo.
- Single-flight: concurrent identical GETs share one request.
- Events are queued in memory and on disk (`<cacheDir>/backend-events.json`, at most 500, oldest dropped), flushed every 60 s or at 20 events, and on quit with a 1 s cap. Events older than 6 days are dropped before sending (the backend rejects 7 days).
- Disk errors in the queue or caches are caught and logged; they never reach the caller.

## Tasks

### 4.1 Config key
- **Goal:** the app knows the backend URL.
- **Target:** `package.json` key `backendUrl` (empty string until phase 7); read where `feedbackRepo` is read (`grep -n feedbackRepo src/main/*.ts`).
- **Steps:**
  1. Add the key; read it the same way `feedbackRepo` is read.
  2. In development only (`!app.isPackaged`), an `ASSETHUB_BACKEND_URL` environment variable overrides it, and `http://127.0.0.1` is allowed, for the fault test in task 7.3. Packaged builds accept `https:` only.
- **Success criteria:** an empty value disables the client; a packaged build ignores the override.
- **Verify:** no verification needed (4.2 tests the disabled case).

### 4.2 Client with budget, breaker, single-flight
- **Goal:** `BackendClient` with injectable `fetch` and clock, for tests.
- **Target:** `src/main/backend.ts` — `export class BackendClient { constructor(opts: { url: string; token: () => string | undefined; fetch: typeof fetch; now?: () => number; random?: () => number }); status(): BackendStatus; get<T>(path, opts: { budgetMs: number; parse: (raw: unknown) => T | undefined }): Promise<T | undefined>; send(path, body, opts): Promise<Response | undefined> }`. `BackendStatus = 'disabled' | 'online' | 'offline'` in `src/shared/ipc.ts`.
- **Steps:**
  1. Every method returns `undefined` instead of throwing.
  2. Add header `X-Asset-Hub-Schema: <REMOTE_CACHE_SCHEMA>` (constant `1` exported from `src/main/backend.ts`).
  3. In production, pass Electron `net.fetch`.
  4. Emit status changes through a listener (`onStatus`).
- **Success criteria:** no code path throws to the caller.
- **Verify:** `npx vitest run src/main/backend.test.ts` passes:
  - "disabled when url is empty",
  - "hang returns undefined at budget" (fetch never resolves and ignores abort; fake timers; result within budget + 50 ms),
  - "slow body counts against budget",
  - "refuses http url and redirects",
  - "DNS error counts as failure",
  - "401 and 403 do not open breaker",
  - "429 and 500 open breaker after 3",
  - "corrupt JSON returns undefined",
  - "half-open probe closes breaker on success",
  - "single-flight shares one request",
  - "never logs the token".

### 4.3 Event queue
- **Goal:** events survive offline time and app restarts, bounded.
- **Target:** `src/main/backend.ts` — `class EventQueue { add(repo, e); flush(); }`; file written with `writeAtomic` from `src/main/caches.ts`.
- **Steps:**
  1. Event id = `crypto.randomUUID()`; group by repo; POST to `/v1/repos/<repo>/events` in chunks of 100.
  2. Drop queued events for a repo on 403 `noAccess`. Keep them on breaker-open, 503 or other 5xx. Drop the ids the backend lists in `rejected`.
  3. Cap at 500; drop oldest.
- **Success criteria:** queue persists and drains.
- **Verify:** `npx vitest run src/main/backend.test.ts -t queue` passes "persists across restart", "caps at 500", "drops on 403", "keeps on 503", "drops rejected ids", "drops events older than 6 days", "clears on account change", "survives unwritable queue file".

### 4.4 Status IPC and quiet indicator
- **Goal:** the user can see that team features are offline, without an alert.
- **Target:** `src/shared/ipc.ts` (`backendStatus: 'backend:status'`, `backendStatusChanged: 'backend:statusChanged'`, preload API `backend: { status(); onStatus(cb) }`), `src/preload/index.ts`, `src/main/github.ts` (create the client in `registerGitHubIpc` after sign-in; register the handler), new `src/renderer/src/components/TeamStatus.tsx` rendered beside `VersionBadge`, locale keys `team.online`, `team.offline`, `team.offlineHint` in `src/renderer/src/i18n/locales/{en,vi,ko}.json`.
- **Steps:**
  1. `online`: render nothing. `offline`: a small muted dot with tooltip `team.offlineHint` ("Team features are offline. Browsing and previews still work."). `disabled`: nothing.
  2. Translate the three keys into Vietnamese and Korean.
- **Success criteria:** indicator appears only when offline.
- **Verify:** `npm test` exits 0 (includes `src/renderer/src/i18n/locales.test.ts`), and `npm run typecheck` exits 0.

## Success Criteria

- `npm test && npm run typecheck && npm run build` exit 0.
- With `backendUrl` empty, a full browse session makes no network call to the backend (test "disabled when url is empty" asserts `fetch` not called).

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
