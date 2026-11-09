---
title: "Phase 6: Team popularity, popular-first order, prefetch, timings"
status: todo
---

# Phase 6: Team popularity and prefetch

## Overview

Popular = team views + sends to Unity, recent-weighted (phase 3). Popular assets sort first when browsing. Because the thumbnail queue (`src/renderer/src/useThumbnailQueue.ts`) draws on-screen cards top-first, they also draw first. The top 20 per branch get their dependencies resolved in the background, so opening them is fast. Without the backend, the order falls back to branch count, then personal history.

## Requirements

- Browse order default = popular-first. Search: match strength first; popularity only breaks ties.
- Fallback with no team data: `branchCount` (catalog scope, as `rankForPlayable` uses in `src/core/checks/playable.ts`), then personal history (`RecentLists.viewed`, `RecentLists.exported` in `src/shared/collections.ts`, and favorites).
- Prefetch is polite, not preemptive. `resolve` (`src/core/github/source.ts:326`) has no cancellation, and git work is serialized per repo (`serially` in `src/main/gitTree.ts`). Prefetch therefore:
  - runs at most one `resolve` at a time;
  - starts the next one only when `foreground === 0`;
  - checks its abort signal between assets, never mid-call. An in-flight resolve finishes; that is at most one asset's worth of delay.
  - uses mode `'quick'` first: the `.meta` files nearest the model, seconds instead of a full GUID index. It then uses `'full'` only if the user is still idle.
  - stops on a rate-limit error.
- Preview timing buckets in usage counts, so the 2–4 s target can be checked from telemetry.

## Tasks

### 6.1 Public SHA lookup and team events
- **Goal:** views and sends reach the backend queue with repo and blob SHA.
- **Target:** `src/core/github/source.ts` — new public `async blobShas(repo: string, branch: string, paths: readonly string[]): Promise<Record<string, string>>` built on the private `snapshot` (line ~760; `byPath` is a `Map`). Also `src/shared/ipc.ts` (`teamView: 'team:view'`, preload `team.view({ repo, sha })`), `src/preload/index.ts`, `src/renderer/src/screens/Browse.tsx` (view timer at line ~978), and `src/main/github.ts` (`IPC.importToUnity` handler at ~340, `IPC.exportPackage` at ~315).
- **Steps:**
  1. Add `blobShas` with a unit test.
  2. In Browse, after the existing `window.api.usage.track({ kind: 'view', ... })`, call `window.api.team.view({ repo, sha })` with the selected item's repo and blob SHA (`catalogSelected.sha` in catalog scope; the `TreeEntry.sha` of `selected` in repo scope).
  3. In main, validate `sha` with `/^[0-9a-f]{40}$/` and `repo` with `/^[\w.-]+\/[\w.-]+$/`, then `EventQueue.add(repo, { kind: 'view', sha })`.
  4. In the send-to-Unity and export handlers, after a successful result, `blobShas(raw.repo, raw.branch, raw.paths)` and add one `send` event per SHA. Do this after replying, not awaited.
- **Success criteria:** one view = one queued event; a send of 3 models = 3 events.
- **Verify:** `npx vitest run src/core/github/github.test.ts -t blobShas` and `npx vitest run src/main/backend.test.ts -t team` pass ("view queues one event", "invalid sha ignored"); `npm run typecheck` exits 0.

### 6.2 Popularity IPC
- **Goal:** the renderer gets `Record<sha, score>` per repo quickly or not at all.
- **Target:** `src/shared/ipc.ts` (`teamPopular: 'team:popular'`, preload `team.popular(repo)`), `src/main/github.ts`, new hook `src/renderer/src/usePopularity.ts`.
- **Steps:**
  1. Main: `client.get('/v1/repos/<repo>/popular', { budgetMs: 1500 })`, cached 10 min per repo; `undefined` or 503 → `{}`.
  2. The hook returns `{}` first and re-renders when scores arrive; it never blocks the first render.
- **Success criteria:** the grid renders before popularity arrives.
- **Verify:** `npm run typecheck` exits 0 (behavior is tested in 6.3 and 7.3).

### 6.3 Popular-first order
- **Goal:** the default browse order puts popular assets first, with the fallbacks above.
- **Target:** new `src/core/checks/popular.ts` — `popularityKey(item, { team, branchCount, personal }): [number, number, number]` and `comparePopular`; `SortMode` in `src/core/checks/playable.ts:201` gains `'popular'`; `Browse.tsx` line 219 default `useState<SortMode>('popular')`; a sort menu option with locale key `sort.popular` in en/vi/ko; search sort uses relevance, then `comparePopular` for ties.
- **Steps:**
  1. Key = (team score or 0, branch count or 0, personal = 2 if favorite + 1 if in recent viewed/exported).
  2. Compare descending by each part, then the existing order.
  3. A user's explicit sort choice still wins (existing state).
- **Success criteria:** order matches the rule in unit tests.
- **Verify:** `npx vitest run src/core/checks/popular.test.ts` passes "team score first", "branch count when no team data", "personal history breaks ties", "search keeps relevance first"; `npm test` exits 0 (includes the locales test).

### 6.4 Background prefetch of the top 20
- **Goal:** the top 20 assets of a newly opened branch open in 2–4 s.
- **Target:** new `src/main/prefetchPopular.ts` — `class PopularPrefetch { constructor(source, isForegroundBusy: () => boolean, waitIdle: () => Promise<void>); start(repo, branch, root, shas: string[]): void; cancel(): void }`. Started through a new IPC `team:prefetch(repo, branch, root)` that `Browse.tsx` calls once popularity for the current repo arrives. The renderer owns `root` (`useState` at `Browse.tsx:206`, set from `findUnityProjectRoots` at ~313); `listTree` does not know it.
- **Steps:**
  1. Map the top 20 SHAs to paths through `blobShas`' snapshot (add a reverse lookup `pathsOfShas` beside it); skip SHAs not in the branch.
  2. Loop: `await waitIdle()`; if aborted, stop; `resolve(repo, branch, path, root, undefined, 'quick')`; then, if still idle, `'full'`. Catch every error; on `rateLimited`, stop the whole run.
  3. `cancel()` on the next `team:prefetch`, a branch switch, or sign-out.
  4. Do **not** add a timeout race around `prefetchBlobs` in `source.prefetch` (line ~722). Abandoning that promise would leave git holding the per-repo lock, so later reads would still wait. If a stuck git fetch shows up in traces, fix it with a subprocess timeout inside `src/main/gitTree.ts` as separate work.
- **Success criteria:** prefetch starts work only while no foreground request runs, and runs at most one at a time.
- **Verify:** `npx vitest run src/main/prefetchPopular.test.ts` passes "max 20", "one at a time", "waits for foreground before each asset", "cancel stops before next asset", "stops on rate limit", "errors do not escape".

### 6.5 Preview timing buckets
- **Goal:** usage reports show how long previews take from selection to visible.
- **Target:** `src/shared/usage.ts` — `UsageEvent` gains `{ kind: 'preview'; ms: number; warm: boolean }`; `noteEvent` (line 53) bumps `preview.ms.<lt2s|2to5|5to10|10to20|gt20>` and `preview.warm` when the asset was prefetched; `Browse.tsx` measures from selection to the viewer's existing loaded callback; `warm` comes from a `team:prefetched(repo, branch)` list.
- **Steps:** add the event; bucket in `noteEvent`; send once per opened asset.
- **Success criteria:** counts appear in the session JSON.
- **Verify:** `npx vitest run src/shared/usage.test.ts` passes "buckets preview ms" (1999 → `lt2s`, 2000 → `2to5`, 25000 → `gt20`).

## Success Criteria

- `npm test && npm run typecheck && npm run build` exit 0.
- With the backend down, browse order equals branch count, then personal history, and nothing waits on the backend.

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
