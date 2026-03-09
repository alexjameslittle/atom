# architecture-review

Validate that changes respect the crate dependency graph and architecture boundaries.

## When to use

Use this skill when a change adds a new dependency between crates, introduces a new crate, or moves
types across crate boundaries.

## Intended dependency flow

```
atom-ffi -> atom-manifest -> atom-modules -> atom-cng -> atom-deploy -> atom-cli
```

`atom-runtime` stays separate from CLI/CNG orchestration code.

## Steps

1. Run `./scripts/architecture-review/check-deps.sh` to extract actual crate dependencies.
2. Compare against the intended flow above.
3. Flag any reverse or unintended cross-layer dependencies.
4. If a new dependency is intentional, require that it be documented in `docs/architecture.md`.

## What to check

- No crate lower in the chain depends on one higher up.
- `atom-runtime` does not depend on `atom-cli`, `atom-cng`, or `atom-deploy`.
- New `use` or `extern crate` statements don't introduce cycles.
- Public types stay in the crate that owns them per the architecture doc.

## Model vs. script split

**Script handles:** extracting dependency edges from Bazel query or Cargo metadata, listing cross-
crate imports.

**Model handles:** judging whether a new dependency is architecturally sound, suggesting where types
should live, proposing dependency direction changes.
