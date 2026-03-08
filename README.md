# Atom

Planning repository for a Rust-first mobile application framework.

The current focus is architecture and tooling, not scaffolding. The formal specification is in [SPEC.md](SPEC.md), and the implementation roadmap is in [docs/plan.md](docs/plan.md).

This branch now bootstraps the Phase 0 and Phase 1 slice from the spec:

- Bazel `bzlmod` toolchain wiring via `.bazelversion`, `MODULE.bazel`, and `mise.toml`
- Bazel-native Rust dependency pinning through `crate_universe` `crate.spec(...)` entries in [`MODULE.bazel`](/Users/alexlittle/conductor/workspaces/atom/tehran/MODULE.bazel)
- public Bazel macros for consumers in [`bzl/atom/defs.bzl`](/Users/alexlittle/conductor/workspaces/atom/tehran/bzl/atom/defs.bzl)
- Bazel rule-driven app and module metadata through `atom_app`, `atom_module`, and `atom_native_module`
- `atom-manifest`, `atom-modules`, `atom-cng`, `atom-runtime`, `atom-ffi`, and `atom-cli`
- a canonical example consumer in [`examples/hello-world`](/Users/alexlittle/conductor/workspaces/atom/tehran/examples/hello-world)
- local and CI verification harnesses driven by `mise`

Current decisions:
- Bazel is the primary build system, using `bzlmod` only.
- `.bazelversion` will be pinned to `8.4.2`.
- `mise.toml` will manage `bazelisk`, `bazel`, and the Rust toolchain.
- Rust dependencies are declared directly in `MODULE.bazel` with pinned `crate.spec(...)` entries. Cargo manifests are not part of the source of truth.
- App and module configuration live in Bazel rules, not in `Atom.toml` sidecars.

The first implementation slice, once planning is approved, is:
1. Toolchain bootstrap with `mise.toml`, `.bazelversion`, `MODULE.bazel`, and `bzl/atom`.
2. A Rust manifest and CNG graph with a dry-run `prebuild`.
3. Thin generated iOS and Android host glue around a Rust app crate.

## Bootstrap

```sh
./scripts/bootstrap.sh
mise run verify
```

The Git hooks in [.githooks](/Users/alexlittle/conductor/workspaces/atom/tehran/.githooks) are installed automatically by the bootstrap task, and GitHub PR verification is defined in [.github/workflows/ci.yml](/Users/alexlittle/conductor/workspaces/atom/tehran/.github/workflows/ci.yml). Rust formatting, Clippy, and tests are all executed through Bazel entrypoints.
