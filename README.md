# Atom

Planning repository for a Rust-first mobile application framework.

The current focus is architecture and tooling, not scaffolding. The formal specification is in
[SPEC.md](SPEC.md), and the implementation roadmap is in [docs/plan.md](docs/plan.md). For
repository conventions and the documentation map, start with [AGENTS.md](AGENTS.md) and
[docs/README.md](docs/README.md).

This branch now implements the Phase 4 runtime/plugin slice from the spec:

- Bazel `bzlmod` toolchain wiring via `.bazelversion`, `MODULE.bazel`, and `mise.toml`
- Bazel-native Rust dependency pinning through `crate_universe` `crate.spec(...)` entries in
  [`MODULE.bazel`](MODULE.bazel)
- public Bazel macros for consumers in [`bzl/atom/defs.bzl`](bzl/atom/defs.bzl)
- Bazel rule-driven app and module metadata through `atom_app`, `atom_module`, and
  `atom_native_module`
- `atom-manifest`, `atom-modules`, `atom-cng`, `atom-runtime`, `atom-deploy`, `atom-ffi`, and
  `atom-cli`
- first-party runtime plugin crates in `crates/atom-navigation` and `crates/atom-analytics`
- kernel-owned runtime state, event/effect, async-task, and Rust module-call plumbing in
  `atom-runtime`
- a canonical example consumer in [`examples/hello-world`](examples/hello-world)
- local and CI verification harnesses driven by `mise`
- generated Swift and Kotlin host bootstraps that start the Rust runtime through
  `atom run --platform ios` and `atom run --platform android`
- app-owned runtime plugin registration through `atom_runtime_config()` in the example app

Current decisions:

- Bazel is the primary build system, using `bzlmod` only.
- `.bazelversion` will be pinned to `8.4.2`.
- `mise.toml` will manage `bazelisk`, `bazel`, and the Rust toolchain.
- Rust dependencies are declared directly in `MODULE.bazel` with pinned `crate.spec(...)` entries.
  Cargo manifests are not part of the source of truth.
- App and module configuration live in Bazel rules, not in `Atom.toml` sidecars.

The hello-world app now proves:

1. Thin generated iOS and Android host glue can boot the Rust runtime.
2. Runtime plugins are registered in app code instead of kernel-side discovery.
3. Navigation and analytics stay as normal library crates outside `atom-runtime`.
4. The runtime can record state changes, run async work, and call a Rust-backed module on both
   platforms.

## Bootstrap

```sh
./scripts/bootstrap.sh
mise run verify
```

The Git hooks in [`.githooks`](.githooks) are installed automatically by the bootstrap task, and
GitHub PR verification is defined in [`.github/workflows/ci.yml`](.github/workflows/ci.yml). Rust
formatting, Clippy, and tests are all executed through Bazel entrypoints.

## Standalone CLI Artifact

GitHub Actions now builds a standalone macOS arm64 `atom` binary artifact in
[`.github/workflows/cli-binary.yml`](.github/workflows/cli-binary.yml). The produced binary can be
downloaded and placed on `PATH`, but Bazel-backed commands still expect `bazelisk` on `PATH` and a
checked-out Bazel workspace rooted by `MODULE.bazel`.
