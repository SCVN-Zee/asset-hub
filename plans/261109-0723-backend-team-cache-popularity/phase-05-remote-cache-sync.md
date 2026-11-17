---
title: "Phase 5: Remote cache sync; retire the GitHub index repo mode (Rust)"
status: done
---

# Phase 5: Remote cache sync

## Overview

Replace the GitHub index repo (`IndexRepo`, `crates/app/src/index_repo.rs`; `INDEX_REPO` at line 19) with per-repo objects on the Worker. The merge logic already exists in `import_index` / `export_index` (`crates/app/src/shared_index.rs:195`, `:285`) over `pub struct Deps { catalog, facts, caches }` (`shared_index.rs:68`). This phase extracts the per-repo parts and reuses them. It does not copy them. Folder mode (`Settings.shared_index_dir`, `SharedIndexService`) stays. With the Worker down, the app uses local caches and GitHub, as today.

Depends on phase 4 (`team::TeamClient`, `Session.team`, `TeamStatus`).

## Requirements

- Pull a repo when a branch of it opens (`browse_assets`, `crates/app/src/commands/parse.rs:426`, and `source_list_tree`, `commands/source.rs:39`), after the local answer is built. Also pull, then contribute, every repo in the local catalog at sign-in and every `CHECK_EVERY` (30 min, `shared_index.rs:42`), one repo at a time. Pull each repo at most once per 10 min.
- Conditional GETs with the stored ETag. Send `If-None-Match` only while the local store still holds data for that object, so a cleared store is filled again.
- Contribute a repo at most every `CONTRIBUTE_AFTER_MS` (2 h, `shared_index.rs:40`), and only when local data adds something. Only `team::writes_enabled()` builds contribute. They stop for the session on 403 `readOnly` (after every account was tried) or on 507.
- Background only: `foreground.wait_idle()` (`crates/app/src/hub.rs:103`) runs before each request batch. Each `pull` / `contribute` has a 5 min whole-operation deadline (`tokio::time::timeout`). Every error is traced and dropped, and no error reaches a command.
- Pull and contribute for one repo never overlap (a per-repo `tokio::sync::Mutex`).
- Personal repos are never uploaded. A repo is personal when its owner equals the login of any signed-in account (`Session.accounts`). This widens `SharedIndexService::include` (`crates/app/src/shared_index_service.rs:131`) to multiple accounts.

### Validation of everything taken in (isolation)

1. **Repo binding:** a catalog fetched for repo R must have `repo` equal to R, ignoring case. A facts object must also have `repo == R`. Otherwise discard it and trace `remote.mismatch`. `CatalogService::merge` (`crates/app/src/catalog_service.rs:590`) trusts the payload's `repo`, so this check runs before it.
2. **Shape:** the catalog goes through `RepoCatalog::parse` (inside `merge`). Facts must be an object keyed by 40-hex SHAs. Stats keys must be 40-hex SHAs. Thumbnail names must match `^[0-9a-f]{40}(\.fx)?$`, and a pack entry that was not requested is dropped.
3. **Membership:** stats, facts and thumbnails are taken only for SHAs in R's trusted `blobs`. Trusted means read from this Mac's local catalog *before* this pull merged anything (`RepoCatalogData.blobs`, `crates/core/src/catalog.rs:45`). A repo this Mac never listed takes in the catalog only. Its other objects come on the next pull.
4. **Size:** the reply body is capped by the client (48 MB). Gunzip runs in `spawn_blocking` with `flate2::read::GzDecoder` and `.take(64 MB + 1)`. A larger output is discarded.
5. **Provenance:** contribute builds objects only from R's own catalog and R's SHAs, so data taken in for one repo is never uploaded under another.

## Tasks

### 5.1 Extract per-repo serializers and merge helpers
- **Target:** `crates/app/src/shared_index.rs`; `crates/app/src/catalog_service.rs` (new `pub async fn blob_shas(&self, repo: &str) -> HashSet<String>` over `load`, line 471).
- **New functions, all `pub(crate)`:**
  - `fn gzip_json<T: Serialize>(value: &T) -> Result<Vec<u8>, String>`, taken out of `write_json_gz` (line 103).
  - `fn catalog_adds(theirs: Option<&Value>, mine: &RepoCatalogData) -> bool`, the `same` test at lines 295–299.
  - `async fn repo_facts(deps: &Deps, kind: &str, repo: &str) -> Result<Option<FactsFile>, String>`, sorted as at lines 322–324. Make `FactsFile` `pub(crate)`.
  - `fn facts_add(theirs: Option<&Map<String, Value>>, known: &Map<String, Value>) -> bool`, which wraps `covers` (line 142).
  - `fn repo_stats(all: &HashMap<String, StatsSummary>, shas: &HashSet<String>) -> BTreeMap<String, StatsSummary>`.
  - `async fn save_newer_cards(deps: &Deps, shared: &HashMap<String, CoreStats>, local: &HashMap<String, StatsSummary>)`, from lines 238–249.
- **Steps:** `export_index` and `import_index` call these helpers, and their output bytes do not change.
- **Verify:** `cargo test -p asset-hub shared_index::` passes the existing tests, plus `per_repo_helpers_match_export_index_files`. `cargo test -p asset-hub --test parity_index` passes.

### 5.2 Thumbnail pack codec
- **Target:** new `crates/app/src/team/pack.rs` — `pub fn encode_pack(entries: &[(String, Vec<u8>)]) -> Vec<u8>` and `pub fn decode_pack(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String>`. The format is repeated `[u16 BE nameLen][utf-8 name][u32 BE byteLen][bytes]` (`backend/src/pack.ts:1`).
- **Steps:** copy the two vectors from `backend/src/pack.test.ts` into Rust tests. Delete the now-unused `ui/shared/thumbPack.ts` and `ui/shared/thumbPack.test.ts`. The page makes no network request, and only `backend/src` comments mention them (`grep -rn thumbPack ui backend/src`). Point the comments in `backend/src/pack.ts:3` and `backend/src/pack.test.ts:7` to `crates/app/src/team/pack.rs`.
- **Verify:** `cargo test -p asset-hub team::pack` passes `encodes_vectors`, `decodes_vectors`, `truncated_is_error`, `bad_utf8_is_error`. `cd backend && npm test` passes.

### 5.3 `RemoteCache::pull`
- **Target:** new `crates/app/src/team/remote_cache.rs`. `pub struct RemoteCache` is built by `RemoteCache::new(client: Arc<TeamClient>, deps: Deps, foreground: Foreground, logins: Vec<String>, state_dir: PathBuf)` and has `pub async fn pull(&self, repo: &str) -> Counts`. Per-repo state lives in `<cacheDir>/remote-cache/<repo_slug>.json` (`repo_slug`, `catalog_service.rs:52`), written with `write_atomic`. It holds the ETag and JSON SHA-256 per object, the thumbnail cursor, `last_contributed` and the remote thumbnail names.
- **Steps:**
  1. Read R's trusted blob set first (`CatalogService::blob_shas`).
  2. GET `cache/catalog`, `cache/stats` and `cache/facts-<kind>` for each kind in `SHARED_FACT_KINDS` (`crates/core/src/github/source.rs:156`), with the `BACKGROUND` budget. A 304 or 404 skips the object.
  3. Validate as above. Then merge with `deps.catalog.merge`, `deps.facts.merge_shared_facts(kind, repo, facts)` and `deps.caches.stats.import_many`. A failed object is skipped and traced, and the rest continue.
  4. Do thumbnails before the stats merge, as `import_index` does (lines 210–249). Read the local stats first. Page `GET thumbs?after=<rev>:<name>&limit=1000` from the saved cursor (items are `{ name, size, rev }`, `backend/src/thumbs.ts:29`). Keep names whose key is trusted and that are missing locally or `newer_card(remote, local)` (`asset_hub_core::stats::newer_card`). Chunk each page in listing order into groups of 40, and POST `thumbs/get` `{ "names": [...] }` for each group's wanted names. Use `stream::iter(..).buffered(2)`, so the order is kept, and wait for idle before each group. Decode with `pack::decode_pack`, then call `deps.caches.thumbs.import_file(name, bytes, replace)`. Save the cursor (the group's last `rev:name`) after the group is stored, and add the names to the remote set.
  5. Then `import_many` the stats and call `save_newer_cards`.
  6. Stop at once on the deadline or on `TeamError::{Offline, Disabled, Timeout, Network}`.
- **Verify:** `cargo test -p asset-hub team::remote_cache::pull` passes `fills_empty_stores`, `not_modified_skips_merge`, `corrupt_object_skipped`, `catalog_for_other_repo_rejected`, `stats_for_sha_outside_repo_ignored`, `oversized_gzip_rejected`, `newer_card_replaces_thumb`, `cursor_saved_after_group`, `stops_when_offline`, `waits_for_idle_before_group`, `unrequested_pack_entry_dropped`. The tests run against `test_server::serve_full`.

### 5.4 `RemoteCache::contribute`
- **Target:** `crates/app/src/team/remote_cache.rs` — `pub async fn contribute(&self, repo: &str) -> Counts`.
- **Steps:**
  1. Skip the repo in these cases: writes are off or paused, the repo is personal, it was contributed within 2 h, or `readOnly` was answered this session.
  2. For the catalog, stats (R's SHAs only) and each facts kind: build the JSON. Skip the object when `catalog_adds` / `facts_add` / the stats canonical compare says the remote object already covers it, or when its SHA-256 equals the last pulled or uploaded value. Otherwise PUT `cache/<name>` with gzip bytes and `If-Match: <etag>` (or `If-None-Match: *` when no ETag is known). On 412, pull that object, merge, rebuild and retry, at most `PUSH_ATTEMPTS` (4, `shared_index_service.rs:25`) times.
  3. Thumbnails: take local names whose key is in R's `blobs`. Leave out remote names, unless the local card is newer than the remote one. Upload in packs of at most 20 entries and 4 MB, at most 2,000 per session, waiting for idle before each pack. Skip entries over 1 MB.
- **Verify:** `cargo test -p asset-hub team::remote_cache::contribute` passes `conflict_then_merge_then_succeed`, `gives_up_after_4`, `skips_personal_repo_of_any_account`, `skips_covered_objects`, `stops_after_read_only`, `stops_after_storage_full`, `uploads_only_this_repos_shas`, `dev_build_uploads_nothing`.

### 5.5 Wire in, gate, then retire the GitHub index repo mode
- **Status:** wiring and the live-check test are done. The Retire step is deferred until the live read-only check has run. The index repo mode is still in place, so the final `git grep` Verify below does not pass yet.
- **Wire:** `Session` gains `pub remote_cache: Arc<RemoteCache>` (`hub.rs:39`, built in `services`, line 188). `Hub::begin` starts `remote_cache.start(CHECK_EVERY)`, which follows the `start_checks` pattern. `Session::end` stops it and aborts the running task (`AbortHandle`). `browse_assets` and `source_list_tree` call `session.remote_cache.schedule_pull(&repo)` after the local result: it spawns, is not awaited, and is rate-limited.
- **Gate (before deleting anything):** 5.1–5.4 pass. A read-only live check passes: `ASSETHUB_LIVE_TOKEN="$(gh auth token)" ASSETHUB_LIVE_REPO=<owner/repo> cargo test -p asset-hub team::remote_cache::live -- --ignored --nocapture`. It is an `#[ignore]` test against `TEAM_API`: health is OK, and `pull` returns with no `Status` error other than 404.
- **Retire:** delete `crates/app/src/index_repo.rs`, `crates/app/tests/parity_index_repo.rs`, and `pub mod index_repo` (`lib.rs:27`). In `shared_index_service.rs`, remove `repo`, `auto_contribute`, `github()`, `resolve_writer`, `read_only`, `should_contribute` and the push loop in `write`, and delete the GitHub-mode tests. With no folder picked, the service is idle. In `hub.rs`, remove `index_remote`, `with_index_remote` and the `IndexRepo` construction (lines 216–217). In `commands/caches.rs:63`, remove the `resolve_writer` call. In `dto.rs:844–862`, remove `SharedIndexVia::Github` and `SharedIndexStatus.{repo, read_only, writer}`. Then `bun run bindings`.
- **UI:** `ui/shared/ipc.ts` types. In `ui/src/screens/Settings.tsx`, `SharedIndexSetting` (line 369) shows "Team cache: online / offline" from `team.status()` in place of the GitHub repo text. Delete `settings.sharedIndexNone`, `sharedIndexNoGit`, `sharedIndexAuto`, `sharedIndexReadOnly`, `sharedIndexReadOnlyMany` and `sharedIndexWriter` from en/vi/ko, and add `settings.teamCacheOnline` and `settings.teamCacheOffline`.
- **Docs:** in `docs/architecture.md`, the read-only row and the "Share the catalog through a GitHub repository" row name `IndexRepo`. Change them to the folder plus the team API.
- **Verify:** `git grep -n "INDEX_REPO\|IndexRepo\|index_repo" crates ui docs/architecture.md` prints nothing. `cargo test --workspace`, `bun run test` and `bun run typecheck` exit 0.

## Success Criteria

- With a fresh cache folder and a reachable Worker, opening a branch fills its catalog and thumbnails without git or the index repo.
- With the Worker unreachable, `[ipc]` times of `browse_assets` and `source:list-tree` under `ASSETHUB_TRACE=1` are within 10 % of `ASSETHUB_BACKEND_URL=` (disabled).

## Risks

- **A teammate plants bad data (low × high).** The validation rules above apply, and the rejection tests cover each rule.
- **Old 2.x builds still write only the index repo (high × low).** Data converges once teammates update. See open question 2.
- **Retiring too early (medium × high).** The gate in 5.5 comes first, and retirement is its own commit.

## Rollback

Revert the retirement commit (5.5) to restore folder and GitHub modes. Delete `<cacheDir>/remote-cache/` to forget sync state. Local stores only ever gain validated data.

## Failure Protocol
If any Verify step does not meet its stated pass condition, STOP this phase.
Do not improvise a fix, retry blindly, or reason around the failure.
Spawn the `kongming` subagent for next-step counsel and pass the phase and task id, the steps you ran, the exact command and its full output, and the pass condition it failed to meet.
Apply kongming's guidance, then re-run the Verify step.
If `kongming` cannot be spawned in this environment, STOP and report the same failure evidence to the user. Never continue by self-reasoning.
