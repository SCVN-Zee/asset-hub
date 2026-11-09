---
title: Team backend Worker built and reviewed
date: 2026-11-09
summary: Phases 2-3 of the team backend plan pass 93 tests after two kongming reviews; app-side phases wait on the Tauri migration decision.
---

# Team backend Worker built and reviewed

## What happened
- Built the Cloudflare Worker (`backend/`) for phases 2-3 of `plans/261109-0723-backend-team-cache-popularity/` in worktree `/Volumes/Supercent/Projects/asset-hub-team-backend`, branch `feat/team-backend`, not committed.
- The Worker covers auth with a 5-minute access cache, push required for writes, events in D1, popularity in Redis rebuilt by cron, cache objects in R2 with ETag writes, and thumbnail packs.
- Checks: 93 backend tests pass, the backend type check is clean, and the root `tsc` is clean.
- The first kongming checkpoint was a no-go. It found:
  - body limits checked after buffering;
  - 40-thumbnail responses that could pass the 128 MB isolate limit;
  - a time-based cursor that could miss replacements;
  - a 45-repo rebuild cap that starved 6 repos;
  - event IDs that were globally unique instead of per repo;
  - `If-Match: *` bypassing the version check.
- All six were fixed with regression tests, and the re-check was a go. Two follow-ups were then fixed: closing a body on cancel during a pending R2 lookup, and measuring real pipeline bytes.
- The fixes changed the thumbnail list contract. Items carry `rev`, and the cursor is `after=<rev>:<name>`. Plan phases 3 and 5 were updated.

## Problem found
Another session started a Tauri + Rust + bun migration in the main checkout (`feat/tauri-bun-migration`, uncommitted edits). My `git switch -c` moved that checkout off its branch 11 s after it switched. I switched it back with no loss and moved all work to a separate worktree. Lesson: check `git reflog` and `git status` for foreign activity before switching branches in a shared checkout.

## Decision pending
Phases 4-6 edit `src/main`, `src/core`, `src/shared/ipc.ts` and preload. The Tauri plan freezes those files and recommends building these features in Rust after parity. The user must choose: Rust later, or Electron now before the freeze.

## Next steps
- Get the user's decision on phases 4-6.
- Phase 7 needs a company Cloudflare account, an Upstash account, and the real D1 `database_id`, plus a staging smoke test of bindings, cron and real Redis pipelines.

> Historical work record — not durable authority. Prefer docs/specs/ADRs for current decisions.
