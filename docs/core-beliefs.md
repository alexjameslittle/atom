# Core Beliefs

## Agent-First Defaults

- Repository knowledge beats oral tradition. If a rule matters, write it down.
- A single verification path beats bespoke local workflows. Local and CI checks should stay aligned.
- Bazel-first is a product decision, not a temporary implementation detail.
- Generated outputs are framework-owned. Customization should flow through metadata and rules, not
  manual edits.
- Determinism matters. Plans and generated trees should be stable for the same inputs.
- Thin interfaces age better than clever ones. CLI glue should stay thin; metadata loaders should
  stay boring.

## Documentation Rules

- Short entrypoints first, deeper docs second.
- Prefer a docs index over scattered long-form notes.
- Design decisions that constrain future changes belong in `docs/design-docs/`.
- If code and docs disagree, fix both in the same change or explicitly note the gap.
