---
title: "Team backend: remote cache and popularity"
description: "Add a Cloudflare Worker backend (D1, R2, Upstash Redis) for a team cache and popular-first browsing; the app keeps working without it."
status: in-progress
priority: P1
effort: "L (about 7–10 working days)"
tags: [backend, performance, cloudflare, redis]
created: 2026-11-09
---

# Team backend: remote cache and popularity

## Overview

Asset Hub has no backend today. Teammates share listings, measurements and thumbnails through the GitHub repo in `INDEX_REPO` (`src/main/indexRepo.ts`), which is **public**. This plan adds a small backend so that:

- the team cache moves to private storage and is shared per repo, behind a GitHub access check;
- popular assets (team views + sends to Unity) sort first when browsing, draw their thumbnails first, and have their dependencies fetched ahead of time;
- when the backend, Redis, or both fail, the app works exactly as today, minus the team features and the remote cache.

Stack (from `plans/reports/researcher-261109-1416-backend-free-tier-services.md`): Cloudflare Workers + D1 + R2 + Upstash Redis, all on free tiers.

## Goals

| # | Goal | Priority |
|---|------|----------|
| 1 | A first branch open on a fresh Mac uses the team cache from private storage | P1 |
| 2 | Popular assets sort first, draw first, and open in 2–4 s | P1 |
| 3 | Backend or Redis outage never blocks or slows local results | P0 |

## Architecture

```mermaid
flowchart LR
  App[Asset Hub main process] -->|GitHub token, TLS, 1.5 s budget, breaker| W[Worker /v1]
  W -->|token + repo access check, 5 min TTL| GH[GitHub API]
  W --> R[(Upstash Redis: rankings, access cache)]
  W --> D[(D1: events, thumb index)]
  W --> R2[(R2 private: catalogs, facts, stats, thumb packs)]
  App -.->|backend down| L[Local caches + GitHub, as today]
```

Rules every phase keeps:

- The Worker never stores or logs the GitHub token; it keeps only `sha256(token)` as a cache key.
- Every response is checked against the caller's access to that repo.
- D1 is the source of truth for events; Redis rankings are derived and rebuilt by cron every 3 hours. If Redis is down, popularity is off and the app uses its local order.
- Reading a repo's cache needs read access to that repo on GitHub; writing needs push access. The desktop validates everything it takes in (repo binding, shape, size, and SHAs in that repo).
- The desktop never waits for the backend before showing local results.

## Phases

| # | Phase | Status | Depends on |
|---|-------|--------|------------|
| 1 | [Shared index access (handled by the owner)](./phase-01-start.md) | Skipped | — |
| 2 | [Backend foundation: Worker, bindings, auth](./phase-02-backend-foundation.md) | Done | — |
| 3 | [Backend data API: events, popularity, cache objects, thumbs](./phase-03-backend-data-api.md) | Done | 2 |
| 4 | [Desktop backend client with fail-open behavior](./phase-04-desktop-backend-client.md) | Deferred (Rust, after Tauri parity) | 3 (contract) |
| 5 | [Remote cache sync; retire the GitHub index repo mode](./phase-05-remote-cache-sync.md) | Deferred (Rust, after Tauri parity) | 4 |
| 6 | [Team popularity, popular-first order, prefetch, timings](./phase-06-team-popularity-and-prefetch.md) | Deferred (Rust, after Tauri parity) | 4 |
| 7 | [Deploy, real-app fault test, docs](./phase-07-deploy-verify-docs.md) | Pending | 5, 6 |

Phase 1 is out of scope: the owner moves the index repo and manages its access. Phases 4–6 wait for the Tauri + Rust migration (`plans/261109-1425-tauri-rust-bun-migration/`): they will be built in Rust against `src-tauri/` after its parity phase, not in the Electron code that migration freezes. The backend contract from phases 2–3 stays as built; note that thumbnail list items carry `rev` and the cursor is `after=<rev>:<name>`. Phases 5 and 6 touch different files except `src/main/github.ts`; run them one after the other, not in parallel.

## Success Criteria

- [ ] `cd backend && npm test` passes, including revoked-access and cross-repo tests.
- [ ] `npm test && npm run typecheck && npm run build` pass at the repo root.
- [ ] Fault tests (hang, DNS failure, 401/403/429/500, corrupt payload) show local results with no added wait beyond the 1.5 s budget, and a "team features offline" indicator.
- [ ] Browse defaults to popular-first; search keeps match strength first.
- [ ] After prefetch, a top-20 asset on a branch opened for the first time shows its preview within 4 s of being selected (task 7.4).
- [ ] Worker, D1, R2 and Upstash usage stays inside the free tiers in the first week.

## Review

An independent review on 2026-11-09 (kongming, gpt-6-astra, medium; receipt MATCH) returned no-go with 5 high findings. All were applied:
- validation of remote data before merging (phase 5);
- corrected references: the `prefabfx3` fact kind, the private `snapshot`, and the unexported `Deps`;
- prefetch limited to what it can control, with no timeout race around the git lock;
- whole-operation deadlines, plus clean-up on account change;
- Redis pipelining, and no D1 scan on a Redis outage.

Medium findings were applied too:
- a composite thumbnail cursor with version-aware replacement;
- rate-limit handling, and push access required for writes;
- a cron-only rebuild, so there is no rename race;
- honest pass conditions in phases 1 and 7.

## Assumptions (defaults; confirm or change before phase 2)

The question popup was declined, so these defaults were chosen. Each is easy to change.

1. "Accessibility feature" means the backend-only team features: team popularity, popular-first order, and team prefetch. Without the backend, order falls back to branch count, then personal history (recent and favorites).
2. Teammates are trusted cache writers, as with today's index repo; writing needs push access to the repo.
3. The owner moves the shared index repo to their own GitHub and manages its access outside this plan. The app keeps using it until phase 5 ships; the shared-folder mode stays.
4. Revoked GitHub access stops working on the backend within 5 minutes (access cache TTL 300 s, no sliding expiry).
5. A team owner creates a company-owned Cloudflare account and an Upstash account.
6. Popularity counts are anonymous per repo and asset (no login stored with them), weighted to recent use (14-day half-life), refreshed every 3 hours, and prefetch is capped at the top 20 per branch.
7. `ALLOWED_OWNERS` = `supercent-io`, `SCVN-Dan` (the GitHub owners whose repos the team browses). Check the list against the repos in the catalog before deploy.

## Open questions

- Does the company already have a Cloudflare account?
- Which GitHub owners belong in `ALLOWED_OWNERS`?
- Should views by the developer's own account count toward popularity?

<!-- slug: backend-team-cache-popularity -->
