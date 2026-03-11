---
name: spec-sync
description:
  Compare implementation facts against SPEC.md so behavior, error codes, lifecycle states, and
  metadata remain documented accurately.
---

# spec-sync

Audit SPEC.md against the current implementation to find contradictions, missing coverage, or
outdated sections.

## When to use

Use this skill when:

- Behavior changes land that aren't reflected in SPEC.md.
- New error codes, lifecycle states, or module metadata fields are added.
- A release candidate is being prepared.

## Steps

1. Run `scripts/extract.sh`.
2. Compare extracted data against SPEC.md sections.
3. Report contradictions (spec says X, code does Y) and gaps (code does Z, spec is silent).

## What to check

- Error codes in `atom-ffi` match the error taxonomy in SPEC.md.
- Lifecycle states in `atom-runtime` match the state machine in SPEC.md.
- Module metadata fields in `atom-modules` match the module manifest section in SPEC.md.
- Exit codes used by `atom-cli` match the exit code table in SPEC.md.
- Backend registry invariants in SPEC.md still match `atom-backends`, `atom-cng`, and `atom-deploy`,
  including the requirement that generic crates stay backend-neutral and config plugins use
  `contribute_backend(...)`.

## Model vs. script split

**Script handles:** extracting error codes, lifecycle states, metadata fields, and exit codes from
source. Extracting the corresponding sections from SPEC.md.

**Model handles:** semantic comparison between implementation and spec, judging whether differences
are bugs or intentional divergence, drafting spec updates.
