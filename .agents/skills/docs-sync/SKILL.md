---
name: docs-sync
description:
  Audit repository docs for stale paths, missing coverage, or contradictions after API,
  architecture, or workflow changes.
---

# docs-sync

Audit documentation against the codebase to find stale, missing, or contradictory docs.

## When to use

Use this skill when a change modifies public API surface, adds or removes crates, changes
architecture boundaries, or updates the plan/spec.

## Steps

1. Run `scripts/audit.sh`.
2. Review the output against the current docs index at `docs/README.md`.
3. Flag any of the following:
   - Crate modules not mentioned in `docs/architecture.md`
   - Public types or traits added without corresponding doc updates
   - Links in docs that point to non-existent files
   - `SPEC.md` sections that contradict current implementation
   - `AGENTS.md` instructions that reference removed or renamed paths
   - Docs that place concrete iOS/Android behavior back into `atom-backends`, `atom-cng`, or
     `atom-deploy` instead of backend crates

## Model vs. script split

**Script handles:** listing public exports, checking link targets, diffing doc references against
file system.

**Model handles:** judging whether a doc gap matters, drafting missing sections, deciding if a spec
contradiction is a doc bug or an implementation bug.
