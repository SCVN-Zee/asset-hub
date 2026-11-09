---
title: "Phase 7: Deploy, real-app fault test, docs"
status: todo
---

# Phase 7: Deploy, verify, docs

## Overview

Create the accounts, deploy the Worker, point a test build at it, prove fail-open behavior in the real app, then release and record the decision.

## Tasks

### 7.1 Accounts, resources, secrets
- **Goal:** production resources exist; no secret in git.
- **Target:** Cloudflare and Upstash dashboards; `backend/wrangler.toml` (IDs only).
- **Steps (team owner):**
  1. Create a company-owned Cloudflare account and an Upstash account (free tiers; no card needed).
  2. `cd backend && npx wrangler login`; `npx wrangler d1 create asset-hub`; `npx wrangler r2 bucket create asset-hub-cache`; put the D1 `database_id` and bucket name in `wrangler.toml` (IDs are not secrets).
  3. Create an Upstash Redis database (region near Singapore); `npx wrangler secret put UPSTASH_REDIS_REST_URL` and `... UPSTASH_REDIS_REST_TOKEN`.
  4. `npx wrangler d1 migrations apply DB --remote`.
- **Success criteria:** resources listed in both dashboards; usage alerts or a weekly check set up against the free limits (Workers 100K requests/day, Upstash 500K commands/month, D1 5M rows read/day).
- **Verify:** `npx wrangler secret list` shows both names; `git grep -n "UPSTASH_REDIS_REST_TOKEN\s*=" ` prints nothing.

### 7.2 Deploy and smoke test
- **Goal:** the Worker is live and enforces auth.
- **Target:** `backend/` deploy; the Worker URL.
- **Steps:**
  1. `cd backend && npm test && npx wrangler deploy --dry-run && npx wrangler deploy`.
  2. `curl -s https://<worker>/v1/health`.
  3. `curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $(gh auth token)" https://<worker>/v1/repos/<owner>/<repo>/popular` for a readable repo, a repo of a non-allowed owner, and with no header.
- **Success criteria:** health returns `{"ok":true,...}`; the three calls return `200`, `403`, `401`.
- **Verify:** the outputs above match exactly.

### 7.3 Real-app fault test
- **Goal:** prove the app works normally when the backend or Redis fails.
- **Target:** `package.json` `backendUrl` (set to the Worker URL); report `plans/261109-0723-backend-team-cache-popularity/reports/fault-test.md`.
- **Steps:** `npm run dev` with `ASSETHUB_TRACE=1`. For each case, open (a) a branch and prefab already cached on this Mac and (b) a branch never opened on this Mac:
  1. normal;
  2. `backendUrl` host that does not resolve (DNS failure);
  3. a local stub server on `http://127.0.0.1:<port>`, selected with the dev-only `ASSETHUB_BACKEND_URL` (task 4.1), that hangs, then returns 500, 429, 401, 403 and corrupt JSON in turn;
  4. Worker reachable, Redis token invalid (`wrangler secret put UPSTASH_REDIS_REST_TOKEN`, restore afterwards);
  5. network off after the grid loads.
- **Success criteria:**
  - In cases 2–4, both (a) and (b) show the grid and preview, and `[ipc]` times for `listTree` are within 10 % of the `backendUrl`-empty baseline. The offline dot shows after the breaker opens.
  - In case 4, cache sync still works and the order falls back to branch count.
  - In case 5, (a) previews from local caches; (b) is expected to fail as it does today without network.
- **Verify:** the report lists the trace lines per case and states pass or fail for each criterion. Pass = every criterion above is met. 

### 7.4 Speed check
- **Goal:** popular assets open in 2–4 s, selection to visible preview, on a branch opened for the first time.
- **Target:** the `preview.ms.*` and `preview.warm` counts from task 6.5; same report file.
- **Steps:**
  1. On a second Mac (or a fresh `userData` folder), open a branch not opened before.
  2. Open 2 top-20 assets **immediately** (before prefetch can finish) and note the time.
  3. Wait until prefetch is done (≤ 1 min), then open 5 other top-20 assets.
  4. Read the session's usage JSON.
- **Success criteria:** the 5 warmed opens are ≤ 4,000 ms each, selection to visible. Record the 2 immediate opens as the cold baseline; they have no target.
- **Verify:** the report lists the 7 times with warm/cold labels; pass if all 5 warm ones are ≤ 4,000 ms.

### 7.5 Release and docs
- **Goal:** the build ships and the decision is recorded.
- **Target:** `CHANGELOG.md`, `docs/architecture.md`.
- **Success criteria:** release published; ledger row present; the old index repo is the owner's to retire.
- **Steps:**
  1. Add the change to `CHANGELOG.md` (Unreleased), then follow README "Release".
  2. Add a ledger row to `docs/architecture.md`: "Team backend on Cloudflare Workers (D1, R2, Upstash Redis); the app fails open". Why: private per-repo cache behind GitHub access, popularity needs shared counts, free tier fits. Rejected: public index repo (leaked listings), always-on server hosts (sleep or cost). Owners: `backend/`, `src/main/backend.ts`, `src/main/remoteCache.ts`. Replace the shared-index row's GitHub-repo wording.
  3. Tell the owner when every active teammate runs the new build (usage comments show the version), so they can retire the old index repo.
  4. For the first month, check the Cloudflare and Upstash dashboards weekly against free limits.
- **Verify:** `npm test` passes; links in `docs/architecture.md` resolve (`grep -o '](.*)' docs/architecture.md` targets exist).

## Success Criteria

- Fault test and speed check reports exist and pass.
- Release published; architecture ledger updated.

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
