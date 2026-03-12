---
name: prebuild-validation
description:
  Validate Atom's prebuild output by checking the dry-run plan, generated host tree, and determinism
  for CNG-related changes.
---

# prebuild-validation

Validate CNG output by running prebuild in both dry-run and real modes, then inspecting the
generated host tree for correctness.

## When to use

Use this skill when changes affect:

- `crates/atom-cng/` (generation planning or template rendering)
- `templates/` (iOS or Android host templates)
- `bzl/atom/` (Bazel macros that emit module metadata)
- Module metadata fields (`atom_module`, `atom_native_module` attributes)

## Steps

1. Run dry-run prebuild and capture the plan output.
2. Run real prebuild to emit the generated tree.
3. Run `mise exec -- scripts/check-tree.sh`.
4. Compare generated output against expectations for determinism.

## What to check

- Dry-run plan includes all declared modules in dependency order.
- Generated tree contains expected platform directories (ios/, android/).
- Generated Swift/Kotlin files compile (verified by build step).
- Re-running prebuild with the same inputs produces identical output.
- No stale files from previous generations are left behind.

## Model vs. script split

**Script handles:** running prebuild, hashing output tree, diffing successive runs.

**Model handles:** reviewing generated code for correctness, checking that module registration order
matches dependency graph, identifying template rendering bugs.
