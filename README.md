# Atom

Planning repository for a Rust-first mobile application framework.

The current focus is architecture and tooling, not scaffolding. The formal specification is in [SPEC.md](SPEC.md), and the implementation roadmap is in [docs/plan.md](docs/plan.md).

Current decisions:
- Bazel is the primary build system, using `bzlmod` only.
- `.bazelversion` will be pinned to `8.4.2`.
- `mise.toml` will manage `bazelisk`, `bazel`, and the Rust toolchain.
- Rust dependencies will follow the Forge pattern: one `third-party/rust/Cargo.toml` and `Cargo.lock`, imported into Bazel with `rules_rust` `crate_universe`.

The first implementation slice, once planning is approved, is:
1. Toolchain bootstrap with `mise.toml`, `.bazelversion`, `MODULE.bazel`, and `third-party/rust`.
2. A Rust manifest and CNG graph with a dry-run `prebuild`.
3. Thin generated iOS and Android host glue around a Rust app crate.
