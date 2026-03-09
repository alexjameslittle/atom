# Architecture

## Core Rule

Bazel is the only source of truth for build graph shape and app/module configuration.

- App metadata comes from `atom_app(...)`.
- Module metadata comes from `atom_module(...)` and `atom_native_module(...)`.
- Rust dependencies are pinned in `MODULE.bazel`.
- Repo-local Cargo manifests and TOML sidecar manifests are intentionally absent.

## Layering

Dependency direction should move one way:

`atom-ffi` -> `atom-manifest` -> `atom-modules` -> `atom-cng` -> `atom-cli`

`atom-runtime` remains separate from CLI/CNG graph orchestration.

### Layer Responsibilities

- `atom-ffi`
  - Stable error taxonomy
  - FlatBuffer error encoding
  - ABI-adjacent types
- `atom-manifest`
  - Loads Bazel-generated app metadata
  - Validates app/platform/build configuration
- `atom-modules`
  - Loads Bazel-generated module metadata
  - Validates module inputs
  - Resolves dependency order and initialization order
- `atom-cng`
  - Merges app + module configuration into deterministic generation plans
  - Writes the host tree for dry-run or materialized output
- `atom-cli`
  - Maps user commands to Bazel-aware workflows
  - Must stay a thin wrapper, not an alternate build system
- `atom-runtime`
  - Runtime kernel: lifecycle state machine (Created → Initializing → Running →
    Backgrounded/Suspended → Terminating → Terminated)
  - Module lifecycle management: init in dependency order, shutdown in reverse
  - Runtime plugin host API (`RuntimePlugin` trait) for observing lifecycle events and owning
    plugin-local state
  - Tokio `current_thread` async runtime available via `PluginContext`
  - Structured logging via `tracing` at lifecycle transitions
  - Handle-based registry for FFI access from generated native hosts
  - `ensure_running()` gate for CNG-generated per-method module exports
  - Module call dispatch is not the runtime's concern — CNG generates direct per-method FFI exports

## Metadata Flow

1. Bazel macros emit JSON metadata targets alongside app/module targets.
2. `atom-cli` resolves the requested app target.
3. `atom-manifest` loads app metadata from Bazel outputs.
4. `atom-modules` loads module metadata from Bazel outputs and orders dependencies.
5. `atom-cng` produces a deterministic generation plan and optional emitted host tree.

## Boundaries To Preserve

- Keep user-facing error mapping in `atom-ffi`.
- Keep validation close to the loader that owns the data.
- Keep codegen deterministic. Two identical inputs should produce the same plan and output tree.
- Keep examples representative. The hello-world example should exercise real repo conventions, not a
  toy path that bypasses them.

## When Adding A New Layer

- Document the responsibility here.
- Document any new invariants in
  [core-beliefs.md](/Users/alexlittle/conductor/workspaces/atom/tehran/docs/core-beliefs.md) or a
  design doc if the change is architectural.
- Add verification coverage in
  [`scripts/verify.sh`](/Users/alexlittle/conductor/workspaces/atom/tehran/scripts/verify.sh) if the
  new layer changes repo-wide expectations.
