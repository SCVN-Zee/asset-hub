---
title: "Phase 4: Desktop team API client (fail-open, Rust)"
status: done
---

# Phase 4: Desktop team API client

## Overview

One Rust module talks to the Worker. It never slows local work: every call has a deadline, failures open a breaker, and every caller has a local fallback. Team features turn off quietly while the Worker is unreachable and come back on their own. The module is named `team` (not `backend`), because the docs already call the Tauri side "the Rust backend".

Live contract (phases 2–3b, `backend/src/index.ts`): base `https://asset-hub-api.asset-hub-team.workers.dev`, routes `/v1/repos/:owner/:repo/{events,popular,cache/:name,thumbs,thumbs/get,thumbs/put}`, `GET /v1/health`, header `x-asset-hub-schema: <n>` on cache and thumbs routes (`backend/src/http.ts` `readSchema`), `Authorization: Bearer <GitHub token>`. Errors are `{ "error": <code> }`: 401 `auth`, 403 `noAccess` / `readOnly`, 412 `preconditionFailed`, 503 `upstream` / `redisDown` / `storageUnavailable`, 507 `storageFull`.

## Decisions

- **Base URL.** Constant `TEAM_API` in `crates/app/src/team/mod.rs`. Release builds use only the constant (HTTPS). Debug builds (`cfg!(debug_assertions)`) accept `ASSETHUB_BACKEND_URL`: an empty value disables the client, and `http://127.0.0.1:<port>` or `http://localhost:<port>` is allowed for the fault test (phase 7.3). No Settings field: the override is a developer tool, and a Settings field would change `ui/shared/settings.ts` and the bindings.
- **Dev builds read, but do not write.** Debug builds read the live Worker by default. They write team data (events, cache PUTs, thumbnail PUTs) only when `ASSETHUB_INDEX_CONTRIBUTE` is set: reuse `usage_env::auto_contribute()` (`crates/app/src/usage_env.rs:48`). See open question 1.
- **Headless CLI and MCP have no team client.** `Hub::with_api` (`crates/app/src/hub.rs:128`, used by `headless/mod.rs:140` and tests) sets no URL. Only `Hub::new` (`hub.rs:124`, used by `state.rs:41`) sets `team::api_url()`.
- **Token = the account that reads the repo.** The session already holds `Arc<AccountTokens>` (`hub.rs:173`). The client asks `tokens.for_repo(repo)` (`crates/core/src/github/client.rs:265`). It tries the next token on 401 `auth`, 403 `noAccess` or 403 `readOnly`, within the same deadline. On success with a later token, it calls `remember_reader` (`client.rs:272`), as `with_each_token` does for git (`crates/app/src/github_net.rs:158`).

## Requirements

- The token is sent only to the validated base URL, with redirects off (`reqwest::redirect::Policy::none()`), so it never follows a redirect. A 3xx is a failure. Errors use `reqwest::Error::without_url()`, and no trace line or error text contains a token.
- A dedicated `reqwest::Client` with `.no_gzip()`, so cache bodies (`application/gzip`, no `Content-Encoding`) arrive as raw gzip. Reply bodies are read in chunks and capped at 48 MB.
- Budgets: `INTERACTIVE = 1.5 s` (popularity), `BACKGROUND = 30 s` (sync requests). `tokio::time::timeout` wraps the whole call: send, body read and parse. Dropping the future cancels the request, so a hang returns on time.
- Breaker per area (`Area::Data`, `Area::Popular`): open after 3 failures in a row; stay open 60 s ± 20 % jitter; one probe when half-open. Failures: network, timeout, 3xx, 429, 5xx, 507, corrupt body. Not failures: 401, 403, 404, 412, 400, 413, 428.
- 507 `storageFull` also pauses all writes for the session. Then only reads continue.
- Status: `Disabled` (no URL or signed out), `Offline` (any breaker open), `Online` otherwise. Each change is emitted once.
- Events queue: in memory and in `<cacheDir>/team-events.json` (`{ login, events }`), at most 500 (oldest dropped). Flush every 60 s or at 20 events, and on quit with a 1 s cap. Events older than 6 days are dropped before sending (the Worker rejects 7 days, `backend/src/events.ts:7`).
- Disk errors in the queue are traced and never reach a caller. No method panics or returns an error that a caller turns into an `IpcError`.
- Sign-out or account change ends the session's team work (`Session::end`, `hub.rs:60`). A queue file written under another login is discarded on load, so a token refresh for the same login keeps its events.
- Single-flight for interactive GETs lives in the popularity cache (task 6.2). It is the only interactive caller, and background sync is already serialized per repo.

## Tasks

### 4.1 Module, URL and client with budget and breaker
- **Target:** new `crates/app/src/team/mod.rs` and `crates/app/src/team/client.rs`; `crates/app/src/lib.rs` (`pub mod team;` beside `pub mod shared_index;`, line 32).
- **Types:**
  - `pub const TEAM_API: &str = "https://asset-hub-api.asset-hub-team.workers.dev";` and `pub const SCHEMA: u32 = 1;` (sent as `x-asset-hub-schema`).
  - `pub fn api_url() -> Option<String>`, which validates the scheme as in Decisions.
  - `pub fn writes_enabled() -> bool { crate::usage_env::auto_contribute() }`.
  - `pub enum Area { Data, Popular }`. `pub struct Reply { pub status: u16, pub etag: Option<String>, pub body: Vec<u8> }`.
  - `pub enum TeamError { Disabled, Offline, Timeout, Network, Status { status: u16, code: String }, Corrupt }` with `Display` that never prints a token.
  - `pub struct TeamOptions { pub interactive: Duration, pub background: Duration, pub open_for: Duration, pub jitter: bool }` with `Default` = the values above. Tests shorten them.
  - `pub struct TeamClient`, built by `TeamClient::new(url: Option<String>, tokens: Arc<AccountTokens>, options: TeamOptions, on_status: Arc<dyn Fn(TeamStatus) + Send + Sync>)`.
- **Methods:** `pub async fn get(&self, repo: &str, path: &str, area: Area, budget: Duration, if_none_match: Option<&str>) -> Result<Reply, TeamError>`; `pub async fn send(&self, repo: &str, method: reqwest::Method, path: &str, body: Vec<u8>, headers: &[(&str, String)], budget: Duration) -> Result<Reply, TeamError>`; `pub fn status(&self) -> TeamStatus`; `pub fn pause_writes(&self)` and `pub fn writes_paused(&self) -> bool`.
- **Notes:** `repo` is `owner/name`. Encode both parts with `asset_hub_core::github::client::encode_uri_component`. Non-2xx replies parse `{error}` into `TeamError::Status`; 304 is `Ok` with an empty body.
- **Verify:** `cargo test -p asset-hub team::client` passes:
  - `disabled_without_url_makes_no_request`
  - `hang_returns_timeout_within_budget` (a `TcpListener` that accepts and never answers; result within budget + 100 ms)
  - `slow_body_counts_against_budget`
  - `release_rules_refuse_http_and_redirects`
  - `dns_error_is_a_breaker_failure`
  - `auth_and_no_access_do_not_open_breaker`
  - `429_503_507_open_breaker_after_three`
  - `corrupt_json_is_corrupt`
  - `half_open_probe_closes_on_success`
  - `no_access_tries_next_account_and_remembers_it`
  - `error_text_never_contains_token`
  - `popular_breaker_does_not_block_data`
  - Timing tests (`hang_returns_timeout_within_budget`, `slow_body_counts_against_budget`) allow 250 ms of slack over the budget, not 100 ms: the whole suite runs in parallel, and one run missed the 100 ms limit by a few ms on a busy machine. The production budgets are unchanged.

  Extend `crates/app/src/test_server.rs` with `serve_full`, which also passes the method and the body read up to `Content-Length`. Keep `serve` unchanged for its current callers.

### 4.2 Event queue
- **Target:** new `crates/app/src/team/events.rs` — `pub struct EventQueue`, `pub enum TeamEventKind { View, Send }`, `pub fn add(&self, repo: &str, kind: TeamEventKind, sha: &str)`, `pub async fn flush(&self)`, `pub fn start(self: &Arc<Self>)` (60 s loop, `Weak` like `SharedIndexService::start_checks`, `crates/app/src/shared_index_service.rs:328`), `pub fn stop(&self)`.
- **Steps:**
  1. `add` ignores the event when `writes_enabled()` is false or writes are paused. It validates `sha` with `^[0-9a-f]{40}$` and `repo` with `^[\w.-]+/[\w.-]+$`. The id is `uuid::Uuid::new_v4()`, and `ts` is epoch ms.
  2. `flush` groups events by repo and POSTs `/events` as `{ "events": [...] }` in chunks of 100 (`backend/src/events.ts:6`). A 2xx drops the chunk, including the `rejected` ids. A 403 `noAccess` drops that repo's events. A 400 drops the chunk, because it would never pass. Offline, timeout, 5xx and 429 keep the chunk.
  3. Persist with `crate::cache::write_atomic` (`crates/app/src/cache/mod.rs:98`) after each change.
- **Verify:** `cargo test -p asset-hub team::events` passes `persists_across_restart`, `caps_at_500`, `drops_repo_on_403`, `keeps_on_503`, `drops_rejected_ids`, `drops_older_than_6_days`, `other_login_file_is_discarded`, `unwritable_file_does_not_fail`, `dev_build_queues_nothing`.

### 4.3 Session wiring, status command and event
- **Target:** `crates/app/src/hub.rs` (field `team_url: Option<String>` on `Hub`; `Session` gains `pub team: Arc<Team>`, built in `session_with` at line 173 from the same `tokens`; `Session::end` calls `team.end()`; `Hub::begin` (line 242) starts the queue; `#[cfg(test)] fn with_team_api(self, url)`; `Emitters` (line 34) gains `team: Arc<dyn Fn(TeamStatus) + Send + Sync>`); `crates/app/src/team/mod.rs` (`pub struct Team { pub client: Arc<TeamClient>, pub events: Arc<EventQueue> }`, `fn end`); `crates/app/src/dto.rs` (`pub enum TeamStatus { Disabled, Online, Offline }` in a `dto!` block); `crates/app/src/events.rs` (`pub struct TeamStatusChanged(pub TeamStatus)`, transparent); new `crates/app/src/commands/team.rs` with `team_status(state) -> Result<TeamStatus, IpcError>` (no session → `Disabled`); `commands/mod.rs`; `commands/caches.rs` `page_emitters` (line 94); `lib.rs` `collect_events!` (line 64) and `collect_commands!` (line 96); `command_names.rs` (`team_status` in `COMMANDS` and `APP_ONLY_COMMANDS`); `crates/app/capabilities/main.json` (`allow-team-status`).
- **Quit:** in `lib.rs` `RunEvent::Exit` (line 306), add `tokio::time::timeout(Duration::from_secs(1), team.events.flush())` for the current session, if there is one.
- **Verify:** `bun run bindings`, then `cargo test -p asset-hub --test contract` and `cargo test -p asset-hub hub::` pass, including a new `account_change_ends_team_work`.

### 4.4 Quiet indicator
- **Target:** `ui/shared/ipc.ts` (`team: { status(): Promise<TeamStatus>; onStatus(cb) }`), `ui/src/api.ts` (beside `sharedIndex`, line 160), new `ui/src/components/TeamStatus.tsx` rendered after `<VersionBadge />` in `ui/src/screens/Browse.tsx:1173`, keys `team.offline` and `team.offlineHint` in `ui/src/i18n/locales/{en,vi,ko}.json`.
- **Steps:** `Offline` renders a small muted dot with the tooltip `team.offlineHint` ("Team features are offline. Browsing and previews still work."). `Online` and `Disabled` render nothing. Translate both keys.
- **Verify:** `bun run test` and `bun run typecheck` exit 0 (the run includes `ui/src/i18n/locales.test.ts`).

## Success Criteria

- `cargo test --workspace`, `bun run test` and `bun run typecheck` exit 0.
- With `ASSETHUB_BACKEND_URL=` (empty), a browse session makes no request to the Worker (`disabled_without_url_makes_no_request`).

## Risks

- **Breaker flaps on a slow link (medium × low).** Background calls get 30 s, and only the popularity call has 1.5 s.
- **`for_repo` picks an account without access (medium × low).** The next token is tried, and the reader is remembered.

## Rollback

Set `TEAM_API` to `""` (as if disabled) or revert the phase commit. No stored data depends on this phase except `team-events.json`, which is safe to delete.

## Failure Protocol
If any Verify step does not meet its stated pass condition, STOP this phase.
Do not improvise a fix, retry blindly, or reason around the failure.
Spawn the `kongming` subagent for next-step counsel and pass the phase and task id, the steps you ran, the exact command and its full output, and the pass condition it failed to meet.
Apply kongming's guidance, then re-run the Verify step.
If `kongming` cannot be spawned in this environment, STOP and report the same failure evidence to the user. Never continue by self-reasoning.
