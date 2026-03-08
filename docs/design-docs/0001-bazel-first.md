# 0001: Bazel-First Repository Contract

Status: accepted

## Context

Atom is a ruleset and framework, not a Cargo workspace with an optional Bazel wrapper. The build graph, metadata graph, verification flow, and example consumer all need one source of truth.

## Decision

- Bazel with `bzlmod` is the only supported build topology.
- App and module configuration live in Bazel rules.
- Rust dependencies are pinned in `MODULE.bazel` via `crate_universe`.
- Repo-local `Cargo.toml`, `Cargo.lock`, `Atom.toml`, and `Atom.module.toml` are not part of the contract.

## Consequences

- Verification, formatting, linting, and tests should be reachable from Bazel entrypoints.
- The CLI should resolve Bazel targets and Bazel-generated metadata instead of filesystem sidecars.
- Examples must model real consumer usage through Bazel rules.
