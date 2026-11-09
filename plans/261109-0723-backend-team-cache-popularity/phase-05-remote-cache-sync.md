---
title: "Phase 5: Remote cache sync; retire the GitHub index repo mode"
status: todo
---

# Phase 5: Remote cache sync

## Overview

Replace the GitHub index repo with per-repo objects on the backend. The merge logic already exists in `importIndex` / `exportIndex` (`src/main/sharedIndex.ts`) behind `interface Deps { catalog; source; stats; thumbs }` (line ~47, not exported yet). This phase reuses it per repo instead of copying it. The shared-folder mode (`Settings.sharedIndexDir`) stays. When the backend is down, the app uses local caches and GitHub, as before, with no remote cache.

## Requirements

- Pull a repo on sign-in (repos in the local catalog) and when a branch of that repo is opened; at most once per repo per 10 min.
- Conditional GETs with stored ETags (`<cacheDir>/remote-cache-etags.json`), so unchanged objects cost one small request.
- Contribute a repo at most every `CONTRIBUTE_AFTER_MS` (2 h), only when local data adds something, and only when the backend says the user can push.
- Background only: waits while the `foreground` counter in `src/main/github.ts` is above 0; never blocks a foreground request. Each `pull`/`contribute` has a whole-operation deadline (5 min) and is wrapped so no error escapes to an unhandled rejection.
- Personal repos (owner = signed-in login) are never uploaded (the rule in `SharedIndexService.include`).

### Validation of everything taken in (isolation)

Remote objects are written by teammates, so the desktop checks them before merging:
1. **Repo binding:** an object fetched for repo R must have `repo === R` (case-insensitive) inside; facts payloads too. Otherwise discard it and count `remote.mismatch`. This stops a reader of repo A from planting a catalog for repo B (`CatalogService.merge` in `src/main/catalog.ts:148` trusts the payload's `repo`).
2. **Shape:** catalog through `RepoCatalog.parse`; facts as `Record<40-hex sha, value>`; stats keys must be 40-hex SHAs.
3. **Membership:** stats, facts and thumbnails are taken only for SHAs listed in R's trusted catalog `blobs`. Trusted means listed by this Mac from GitHub, from the local catalog *before* this pull merged anything (`RepoCatalogData` in `src/core/catalog/catalog.ts`). A repo this Mac never listed takes in the catalog only. Its stats, facts and thumbnails are taken in on the next pull, after a local listing.
4. **Size:** gunzip with `zlib.gunzip` (async, off the main thread) and `maxOutputLength: 64 MB`; larger → discard.
5. **Provenance:** re-export (contribute) builds objects only from R's own catalog, so data taken in for one repo is never uploaded under another.

## Tasks

### 5.1 Export the merge contract and per-repo serializers
- **Goal:** one function builds each per-repo object; folder mode and remote cache both use it.
- **Target:** `src/main/sharedIndex.ts` — `export interface Deps`; new exports `repoCatalogBytes(deps, repo)`, `repoFactsBytes(deps, kind, repo)`, `repoStatsBytes(deps, shas)`; `exportIndex` calls them.
- **Steps:** export `Deps`; move the per-repo code from `exportIndex` into these functions without changing output bytes (deterministic key order stays).
- **Success criteria:** folder export output is byte-identical.
- **Verify:** `npx vitest run src/main/sharedIndex.test.ts` passes unchanged tests plus "per-repo serializers match exportIndex files".

### 5.2 `RemoteCache.pull`
- **Goal:** a fresh Mac gets a repo's catalog, facts, stats and thumbnails from the backend, validated.
- **Target:** new `src/main/remoteCache.ts` — `class RemoteCache { constructor(client: BackendClient, deps: Deps, opts: { idle: () => Promise<void>; login: () => string | undefined }); pull(repo: string, signal: AbortSignal): Promise<RemoteCacheResult>; contribute(repo: string, signal: AbortSignal): Promise<RemoteCacheResult> }`.
- **Steps:**
  1. Read R's trusted blob set from the local catalog first.
  2. GET `cache/catalog`, `cache/stats`, `cache/facts-<kind>` for each kind in `SHARED_FACT_KINDS` (`src/core/github/source.ts`), with `If-None-Match`.
  3. Validate per "Validation" above, then merge with `deps.catalog.merge`, `deps.source.mergeSharedFacts(kind, repo, facts)`, `deps.stats.importMany`. A failed object is skipped and counted (`remote.corrupt`, `remote.mismatch`); the rest continue.
  4. Thumbnails, after stats (their version check needs stats): page `GET thumbs?after=<rev>:<name>` (items carry `rev`, not a time); keep names whose SHA is in the trusted set and that are missing locally **or newer** per `newerCard(shared, local)` (`src/core/checks/playable.ts`, as `importIndex` does at `sharedIndex.ts:74–90`, including the stats card-version save afterwards). Fetch in batches of 40 with 2 in flight, `await opts.idle()` before each batch; `deps.thumbs.importFile(name, bytes, replace)`. Decode with `src/shared/thumbPack.ts`.
  5. Save the cursor per repo **after** each batch is stored, so a crash re-fetches at most one batch.
  6. Stop at once on `signal.aborted`, the 5-min deadline, or when the client returns `undefined` (backend down).
- **Success criteria:** a pull against a fake client fills an empty cache and rejects mismatched or oversized data.
- **Verify:** `npx vitest run src/main/remoteCache.test.ts` passes "pull fills empty stores", "304 skips merge", "corrupt object skipped", "catalog for other repo rejected", "stats for SHA outside repo ignored", "oversized gzip rejected", "newer card replaces thumb", "cursor saved after batch", "stops when backend down", "waits for idle before thumb batch", "never throws".

### 5.3 `RemoteCache.contribute`
- **Goal:** what this Mac lists and renders reaches teammates.
- **Target:** `src/main/remoteCache.ts` — `contribute`; per-repo state in `<cacheDir>/remote-cache-state.json` (last upload hash and time per object).
- **Steps:**
  1. Skip personal repos, repos contributed within 2 h, and repos where the backend answered `readOnly` (remember for the session).
  2. For catalog, stats (R's SHAs only) and each facts kind: build bytes with 5.1; if the SHA-256 equals the last uploaded or pulled bytes, skip; else PUT with `If-Match` (or `If-None-Match: *` when none). On 412: pull that object, merge, rebuild, retry; at most 4 attempts (`PUSH_ATTEMPTS`).
  3. Thumbnails: local names whose SHA is in R's catalog `blobs`, minus remote names with an equal or newer card → upload in packs of 20, at most 2,000 per session, waiting for idle.
- **Success criteria:** concurrent writers converge; readers without push never upload.
- **Verify:** `npx vitest run src/main/remoteCache.test.ts -t contribute` passes "412 then merge then succeed", "gives up after 4", "skips personal repo", "skips unchanged bytes", "stops after readOnly", "uploads only this repo's SHAs".

### 5.4 Wire it in and retire the GitHub index repo mode
- **Goal:** the backend replaces `INDEX_REPO`; folder mode stays.
- **Target:** `src/main/github.ts` (lines ~113–121 where `SharedIndexService` and `IndexRepo` are created; the `IPC.listTree` handler at ~235; sign-out handling), `src/main/sharedIndex.ts` (`github()`, `include`, the `write` push loop, `readOnly`), delete `src/main/indexRepo.ts` and `src/main/indexRepo.test.ts`, `src/renderer/src/screens/Settings.tsx` (`SharedIndexSetting`), locale keys about the GitHub index repo in en/vi/ko.
- **Steps:**
  1. Create `RemoteCache` after sign-in; in the background, `pull` then `contribute` for catalog repos, one repo at a time.
  2. In the `IPC.listTree` handler, after the local answer is returned, schedule `pull(repo)` (not awaited); abort the previous one when another branch opens.
  3. On sign-out or account change, abort all remote cache work and reset per-session state.
  4. Remove the `IndexRepo` path from `SharedIndexService`: with no folder picked, the service is idle. Keep folder import/export.
  5. Settings shows "Team cache: online / offline" from `BackendStatus` instead of the GitHub repo name. Update en/vi/ko; delete keys no longer used.
- **Success criteria:** no reference to `INDEX_REPO` remains; folder mode tests still pass.
- **Verify:** `git grep -n "INDEX_REPO\|IndexRepo" src` prints nothing, and `npm test && npm run typecheck && npm run build` exit 0.

## Success Criteria

- With a fresh cache directory and a reachable backend, a repo's catalog and thumbnails fill without pulling the index repo.
- With the backend unreachable, `listTree` times under `ASSETHUB_TRACE=1` match the same build with `backendUrl` empty (within 10 %).

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
