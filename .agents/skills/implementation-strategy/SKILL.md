---
name: implementation-strategy
description:
  Collect repo context and draft a concrete edit plan before making a multi-file, architectural, or
  high-risk change in Atom.
---

# implementation-strategy

Build a concrete implementation plan before editing code when the change spans multiple crates,
touches architecture boundaries, or needs a careful verification strategy.

## When to use

Use this skill when:

- The request spans multiple files or subsystems.
- The correct file map is not obvious yet.
- A change might trigger multiple mandatory skills and you need to sequence them cleanly.
- You need to hand the user a tight plan with target files, risks, and verification before editing.

## Steps

1. Run `mise exec -- scripts/collect.sh`.
2. Identify the primary source-of-truth docs, code targets, and verification entrypoints for the
   task.
3. Draft a plan that names the files to change, the invariants to preserve, and the checks to run.
4. Only start editing after the plan is concrete enough to execute.

## Output

- A short implementation plan with target files.
- The verification commands required for the change.
- The likely follow-on skills to run after editing.

## Model vs. script split

**Script handles:** collecting repo state, branch status, key docs, skill inventory, and
verification entrypoints.

**Model handles:** deciding which files matter, sequencing work, identifying risks, and drafting the
implementation plan.
