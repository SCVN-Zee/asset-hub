---
title: "Phase 1: Shared index access (handled by the owner)"
status: skipped
---

# Phase 1: Shared index access (handled by the owner)

## Overview

The owner moves the shared index repo to their own GitHub account and manages who can access it. This plan does not change visibility, collaborators or access for that repo.

Code impact: none in this phase. If the repo name changes before phase 5 ships, the owner updates `INDEX_REPO` in `src/main/indexRepo.ts` in a normal release. Phase 5 removes the GitHub index repo mode, so the name is needed only until then.

## Tasks

None.

## Success Criteria

Not tracked by this plan.
