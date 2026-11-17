---
title: "Phase 6: Team popularity, popular-first order, prefetch, timings (Rust + UI)"
status: done
---

# Phase 6: Team popularity and prefetch

## Overview

Popular means team views plus sends to Unity, weighted to recent use (phase 3, `backend/src/popular.ts`). Popular assets sort first when browsing. `useThumbnailQueue` (`ui/src/useThumbnailQueue.ts`) draws on-screen cards top-first, so popular cards also draw first. The top 20 of a newly opened branch get their dependencies resolved in the background, so they open fast. Without the Worker, the order falls back to branch count, then personal history.

Depends on phase 4 (`TeamClient`, `EventQueue`, `team_status` plumbing). It runs after phase 5 because both edit `hub.rs`, `commands/parse.rs` (`browse_assets`), `lib.rs`, `command_names.rs`, `capabilities/main.json`, `dto.rs`, `ui/shared/ipc.ts`, `ui/src/api.ts`, the bindings and the locales.

## Requirements

- The default browse order is popular-first. Search keeps match strength first, and popularity only breaks ties.
- Fallback with no team data: `branchCount` (catalog scope; the `used` key of `rankForPlayable`, `ui/src/playable.ts:70`), then personal history. Personal history is favorites (`Browse.tsx:225`) and `recent.viewed` / `recent.exported` (`ui/shared/collections.ts:85`).
- Popularity is per repo. In catalog scope, it applies to items of the current `repo`, and other repos score 0 (open question 3).
- Prefetch is polite, not preemptive. `resolve` (`crates/core/src/github/source.rs:316`) has no cancellation, and git work is serialized per repo (`crates/app/src/git_tree.rs`). Prefetch therefore:
  - runs at most one `resolve` at a time, and never enters `Foreground`;
  - starts the next one only after `foreground.wait_idle()` (`crates/app/src/hub.rs:103`);
  - checks a cancel flag between assets, never mid-call, so an in-flight resolve finishes;
  - uses `ResolveMode::Quick` first (`source.rs:115`), then `ResolveMode::Full` only if the foreground is still idle;
  - stops the whole run on `SourceErrorCode::RateLimited`.
- Preview timing buckets in the usage notes, so the 2–4 s target can be checked (phase 7.4).

## Tasks

### 6.1 SHA lookups in the core source
- **Target:** `crates/core/src/github/source.rs` (`impl GitHubAssetSource`, beside `read_file` at line 290). Add `pub async fn blob_shas(&self, repo: &str, branch: &str, paths: &[String]) -> Result<HashMap<String, String>, SourceError>` (path to SHA) and `pub async fn paths_of_shas(&self, repo: &str, branch: &str, shas: &[String]) -> Result<Vec<(String, String)>, SourceError>` (SHA to first blob path, in `shas` order, with missing SHAs left out). Both read `self.inner.snapshot(repo, branch)` (`source.rs:473`; `by_path`, `entries`).
- **Verify:** `cargo test -p asset_hub_core --test github_source shas` passes `blob_shas_of_listed_paths` and `paths_of_shas_skips_unknown`.

### 6.2 Popularity cache and command
- **Target:** new `crates/app/src/team/popular.rs` — `pub struct Popularity`, `pub async fn scores(&self, repo: &str) -> Arc<HashMap<String, f64>>`, and `pub fn top(&self, repo: &str, n: usize) -> Vec<String>` from the cache only. `Team` (`team/mod.rs`) gains `pub popular: Arc<Popularity>`. New command `team_popular(state, repo: String) -> Result<HashMap<String, f64>, IpcError>` in `crates/app/src/commands/team.rs`. Register it as in task 4.3 (`lib.rs`, `command_names.rs`, `capabilities/main.json` `allow-team-popular`), then `bun run bindings`. Add `team.popular(repo)` in `ui/shared/ipc.ts` and `ui/src/api.ts`.
- **Steps:**
  1. `GET popular` with `Area::Popular` and the `INTERACTIVE` budget (1.5 s). The reply is `{ items: [{ sha, score }], builtAt }` (`backend/src/popular.ts:29`). Keep entries whose `sha` is 40-hex and whose `score` is finite and positive.
  2. Cache success per repo for 10 min. A failure returns an empty map and is not cached, because the breaker already limits retries.
  3. Single-flight: concurrent calls for one repo share one request (a per-repo `tokio::sync::Mutex` held across the fetch, with the cache read again after taking it).
  4. The command never fails because of the Worker. Signed out or offline, it returns `{}`.
- **Verify:** `cargo test -p asset-hub team::popular` passes `scores_cached_10_min`, `redis_down_is_empty_and_not_cached`, `concurrent_calls_share_one_request`, `hang_returns_empty_within_budget`, `invalid_items_dropped`.

### 6.3 Team events: views and sends
- **Target:** `crates/app/src/dto.rs` (`UsageEvent::View`, line 1043, gains `repo: Option<String>` and `sha: Option<String>`); `ui/shared/usage.ts` (`UsageEvent`, same fields); `crates/app/src/commands/services.rs` `usage_track` (line 91); `crates/app/src/commands/exporter.rs` (`exporter_unity_package`, line 99, and `exporter_import_to_unity`, line 128); `ui/src/screens/Browse.tsx` (view timer, lines 1038–1048).
- **Steps:**
  1. In Browse, add `repo` and `sha` to the existing `window.api.usage.track({ kind: 'view', type })`. In catalog scope they are `catalogSelected.sha` and `catalogSelected.locations[0].repo`. In repo scope they are `selected.sha` and `repo`.
  2. `usage_track` keeps `note_event` as it is. When both fields are present and a session exists, it also calls `session.team.events.add(repo, View, sha)`. `add` validates the fields and is a no-op in dev builds.
  3. In both exporter commands, after an `Ok` result that is not canceled, spawn a task that is not awaited. It runs `source.blob_shas(&repo, &branch, &paths)` and adds one `Send` event per SHA. Keep `repo`, `branch` and `paths` from the request before `export_input` consumes it.
- **Verify:** `cargo test -p asset-hub team::events` passes the new `view_with_sha_queues_one_event`, `view_without_sha_queues_nothing` and `invalid_sha_ignored`. `bun run bindings && cargo test -p asset-hub --test contract` passes.

### 6.4 Popular-first order
- **Target:** `ui/src/playable.ts` (`SortMode`, line 64, gains `'popular'`; `rankForPlayable` gains a last optional parameter `popularOf: (item: T) => { team: number; personal: number } = () => ({ team: 0, personal: 0 })`); new hook `ui/src/usePopularity.ts`; `ui/src/screens/Browse.tsx` (`useState<SortMode>('relevance')` at line 258 becomes `'popular'`; the `rankedShown` / `rankedCatalog` memos at lines 523–546 pass `popularOf`; the dropdown at line 1329 lists `'popular'` first); key `ranking.sorts.popular` ("Popular with the team") in en/vi/ko.
- **Steps:**
  1. In `rankForPlayable`, handle `'popular'` beside `'used'`, before the `!s` branch, so it needs no measurements. The keys are `[-team, -usedOf(item), -personal]`, after the existing `-matchOf(item)`. Search therefore stays first, and the original index breaks the last ties.
  2. `personal` is 2 when the item is a favorite, plus 1 when it is in recent viewed or exported (`${repo}\n${path}`).
  3. `usePopularity(repo)` returns `{}` first and re-renders when the scores arrive. It never blocks the first render.
  4. A sort the user picks still wins, through the existing state.
- **Verify:** `bunx vitest run ui/src/playable.test.ts` passes the new `team score first`, `branch count when no team data`, `personal history breaks ties` and `search keeps relevance first`. `bun run test` and `bun run typecheck` exit 0.

### 6.5 Background prefetch of the top 20
- **Target:** new `crates/app/src/team/prefetch.rs` — `pub struct Prefetch` built by `Prefetch::new(source: Arc<Source>, popular: Arc<Popularity>, foreground: Foreground)`, with `pub fn start(self: &Arc<Self>, repo: String, branch: String, root: String)`, `pub fn cancel(&self)` and `pub fn was_prefetched(&self, repo: &str, sha: &str) -> bool`. `Session` (`hub.rs:39`) gains `pub prefetch: Arc<Prefetch>`, and `Session::end` cancels it. `browse_assets` (`commands/parse.rs:426`) calls `session.prefetch.start(repo, branch, chosen)` after it builds the reply, when `chosen` is `Some`. Rust picks the root there, so no new command is needed.
- **Steps:**
  1. `start` is idempotent for the same `(repo, branch, root)`. A different key cancels the previous run.
  2. In a spawned task: `popular.scores(repo)`, then the top 20 SHAs, then `source.paths_of_shas`.
  3. For each path: `wait_idle`, check the cancel flag, then `resolve(repo, branch, path, root, None, ResolveMode::Quick)`. If `!foreground.busy()`, also run `Full`. Catch every error. On `RateLimited`, stop the run. Record `(repo, sha)` when Quick succeeds.
  4. Do not add a timeout around `resolve` or `prefetch_blobs`: dropping it mid-call could leave git holding the per-repo lock. If a stuck git fetch shows up in traces, fix it with a subprocess timeout in `git_tree.rs` as separate work.
- **Verify:** `cargo test -p asset-hub team::prefetch` passes `max_20`, `one_at_a_time`, `waits_for_foreground_before_each`, `cancel_stops_before_next`, `stops_on_rate_limit`, `errors_do_not_escape`, `same_key_does_not_restart`. Use a fake `Source` built as in `hub.rs` tests (`test_server::serve`).

### 6.6 Preview timing buckets
- **Target:** `crates/app/src/dto.rs` (`UsageEvent::Preview { ms: f64, repo: Option<String>, sha: Option<String> }`); `crates/app/src/usage.rs` `UsageSession::note_event` (line 99); `commands/services.rs` `usage_track`; `ui/shared/usage.ts`; `ui/src/screens/Browse.tsx`.
- **Steps:**
  1. `note_event` bumps `preview.ms.<lt2s|2to5|5to10|10to20|gt20>`. The bounds are 2,000, 5,000, 10,000 and 20,000 ms, and a negative or non-finite `ms` is ignored.
  2. `usage_track` also bumps `preview.warm` through `state.usage.recorder().with(|s| s.bump("preview.warm", 1))` when `session.prefetch.was_prefetched(repo, sha)`.
  3. Browse starts a timer when the selected asset changes. It sends one `preview` event when `viewerLoading` (line 1131) next turns `false` for that asset. If the selection changes first, it sends nothing.
- **Verify:** `cargo test -p asset-hub usage::` passes `buckets_preview_ms` (1999 → `lt2s`, 2000 → `2to5`, 25000 → `gt20`, -1 → nothing). `bun run typecheck` exits 0.

## Success Criteria

- `cargo test --workspace`, `bun run test` and `bun run typecheck` exit 0.
- With the Worker down, browse order is branch count, then personal history, and no command waits more than 1.5 s on the Worker (`hang_returns_empty_within_budget`).

## Risks

- **Prefetch competes with the user for git (medium × medium).** It runs one resolve at a time, only when idle, and checks cancel between assets.
- **Popular-first surprises users who expect search order (low × low).** Search keeps relevance first, and the dropdown still offers every sort.

## Rollback

Revert the default sort to `'relevance'` (one line in Browse) to keep the data path without the UX change. Revert the phase commit to remove the rest. Team events already sent are only anonymous counts.

## Failure Protocol
If any Verify step does not meet its stated pass condition, STOP this phase.
Do not improvise a fix, retry blindly, or reason around the failure.
Spawn the `kongming` subagent for next-step counsel and pass the phase and task id, the steps you ran, the exact command and its full output, and the pass condition it failed to meet.
Apply kongming's guidance, then re-run the Verify step.
If `kongming` cannot be spawned in this environment, STOP and report the same failure evidence to the user. Never continue by self-reasoning.
