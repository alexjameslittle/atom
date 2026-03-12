---
name: architecture-review
description:
  Check crate dependency direction and type ownership so cross-crate changes stay inside Atom's
  documented architecture boundaries.
---

# architecture-review

Validate that changes respect the crate dependency graph and architecture boundaries.

## When to use

Use this skill when a change adds a new dependency between crates, introduces a new crate, or moves
types across crate boundaries.

## Intended dependency flow

```
atom-ffi -> atom-manifest -> atom-modules -> atom-backends -> atom-cng -> atom-deploy -> atom-backend-{ios,android} -> atom-cli
```

`atom-runtime` stays separate from CLI/CNG orchestration code.

## Steps

1. Run `mise exec -- scripts/check-deps.sh`.
2. Compare against the intended flow above.
3. Confirm generic crates (`atom-backends`, `atom-cng`, `atom-deploy`) stay free of concrete
   first-party backend ids and backend-specific hook names.
4. Confirm backend ids do not replace compatibility fields in machine-readable payloads; destination
   and evaluation descriptors must keep the serialized `platform` field even when `backend_id`
   exists.
5. Confirm `atom run --platform <platform>` still rejects disabled backends before any CNG write
   step or generated-tree mutation.
6. Flag any reverse or unintended cross-layer dependencies.
7. If a new dependency is intentional, require that it be documented in `docs/architecture.md`.

## What to check

- No crate lower in the chain depends on one higher up.
- `atom-runtime` does not depend on `atom-cli`, `atom-cng`, or `atom-deploy`.
- `atom-backends`, `atom-cng`, and `atom-deploy` remain backend-neutral orchestration layers.
- Serialized destination and evaluation payloads preserve compatibility fields such as `platform`
  alongside any additive `backend_id`.
- New `use` or `extern crate` statements don't introduce cycles.
- Public types stay in the crate that owns them per the architecture doc.

## Model vs. script split

**Script handles:** extracting dependency edges from Bazel query or Cargo metadata, listing cross-
crate imports.

**Model handles:** judging whether a new dependency is architecturally sound, suggesting where types
should live, proposing dependency direction changes.
