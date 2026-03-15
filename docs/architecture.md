# Architecture

## Core Rule

Bazel is the only source of truth for build graph shape and app/module configuration.

- App metadata comes from `atom_app(...)`.
- Module metadata comes from `atom_module(...)` and `atom_native_module(...)`.
- Rust dependencies are pinned in `MODULE.bazel`.
- Repo-local Cargo manifests and TOML sidecar manifests are intentionally absent.

## Layering

Dependency direction should move one way:

`atom-ffi` -> `atom-manifest` -> `atom-modules` -> `atom-backends` -> `atom-cng` -> `atom-deploy` ->
`atom-backend-{ios,android}` -> `atom-cli`

`atom-runtime` and runtime support libraries remain separate from CLI/CNG graph orchestration.

### Layer Responsibilities

- `atom-ffi`
  - Stable error taxonomy
  - FlatBuffer error encoding
  - ABI-adjacent types
  - Generated export buffer/codecs helpers used by proc-macro-expanded bridges
- `atom-manifest`
  - Loads Bazel-generated app metadata
  - Validates app/platform/build configuration
- `atom-modules`
  - Loads Bazel-generated module metadata
  - Validates module inputs
  - Resolves dependency order and initialization order
- `atom-backends`
  - Owns backend contracts, shared destination/evaluation/CNG/doctor data types, and generic
    registries
  - Defines the compile-time seam for first-party backend composition without dynamic loading
  - Stays platform-neutral; concrete iOS/Android behavior lives in backend implementation crates
- `atom-cng`
  - Validates framework-wide extension compatibility and delegates backend-specific compatibility
    checks to registered backends
  - Merges app + module + config-plugin configuration into deterministic generation plans
  - Parses Rust module source metadata (`#[atom_record]`, `#[atom_export]`, `#[atom_import]`) and
    emits per-module FlatBuffers schema/build packages for Rust, Swift, and Kotlin bindings
  - Dispatches backend planning and emission through registered `GenerationBackend` contracts
  - Writes generic contributed files plus delegates backend host-tree emission
- `atom-cng-app-icon`
  - First-party config/CNG plugin crate implementing the public `ConfigPlugin` trait
  - Owns app icon config parsing, validation, file contributions, and platform resource wiring
- `atom-deploy`
  - Device discovery and destination selection
  - Dispatches run/stop/evaluate flows through registered `DeployBackend` contracts
  - Owns generic evidence capture, UI evaluation, and proof-bundle orchestration
  - Preserves compatibility fields such as serialized destination `platform`; backend ids are
    additive dispatch data, not replacements for stable machine-readable payload fields
  - Keeps platform deployment orchestration out of `atom-cli`
- `atom-backend-ios`
  - First-party iOS backend implementation crate
  - Registers iOS deploy and CNG backends for the canonical CLI binary
  - Owns iOS destination discovery, deploy/stop/evaluate implementation, and iOS host templates
  - Owns iOS-specific environment doctor probes
  - Owns iOS-specific CNG planning/emission and backend compatibility checks
- `atom-backend-android`
  - First-party Android backend implementation crate
  - Registers Android deploy and CNG backends for the canonical CLI binary
  - Owns Android destination discovery, deploy/stop/evaluate implementation, and Android host
    templates
  - Owns Android-specific environment doctor probes
  - Owns Android-specific CNG planning/emission and backend compatibility checks
- `atom-cli`
  - Maps user commands to Bazel-aware workflows
  - Links the first-party config plugin registry used during `atom prebuild`
  - Builds the canonical first-party backend registries used for deploy/evaluate/CNG composition
  - Aggregates core toolchain checks with backend-owned platform diagnostics for `atom doctor`
  - Exposes uniform backend-aware verbs such as `atom run --platform <platform>`
  - Must stay a thin wrapper, not an alternate build system
- `atom-macros`
  - Provides `#[atom_record]` and `#[atom_export]` as Rust-authored module ergonomics
  - Expands only against stable `atom-ffi` and `atom-runtime` APIs
  - Must not own CNG discovery, schema planning, or backend-specific behavior
- `atom-runtime`
  - Runtime kernel: lifecycle state machine (Created → Initializing → Running →
    Backgrounded/Suspended → Terminating → Terminated)
  - Kernel-owned singleton runtime with inspection snapshots (`RuntimeSnapshot`) for state values,
    dispatched events, completed effects, and recorded lifecycle
  - App-owned runtime config assembly through `atom_runtime_config()` in the app crate
  - Tokio `current_thread` async runtime available through the public `tokio_handle()` free function
  - Public free functions for shared state writes, state reads, event dispatch, and `Running`
    lifecycle checks
  - Structured logging via `tracing` at lifecycle transitions
  - Hidden generated-host entrypoints for singleton init, lifecycle dispatch, and shutdown
- `atom-navigation`
  - First-party library that owns a route stack outside the kernel and may publish route changes
    through `atom_runtime::*` from its public API
  - Proves navigation is a library concern rather than kernel state
- `atom-analytics`
  - First-party library that buffers app-owned analytics events outside the kernel and may publish
    tracking state through `atom_runtime::*` from its public API
  - Proves non-routing headless app behavior composes through the same public runtime free functions

## Metadata Flow

1. Bazel macros emit JSON metadata targets alongside app/module targets.
2. `atom-cli` resolves the requested app target.
3. `atom-manifest` loads app metadata from Bazel outputs.
4. `atom-modules` loads module metadata from Bazel outputs and orders dependencies.
5. `atom-cli` builds the canonical first-party backend registries for CNG and deploy/evaluate.
6. `atom-cng` validates module + config-plugin compatibility, instantiates registered config
   plugins, and produces a deterministic generation plan through registered backend planners.
7. First-party backend crates contribute backend defaults, backend-specific compatibility checks,
   and host-tree emission through the shared CNG contracts.
8. `atom-deploy` resolves destinations, proof plans, and backend sessions through registered backend
   contracts when needed.
9. Generated runtime bridge code links the app crate, calls `atom_runtime_config()`, and passes the
   resulting config to `atom_runtime::__init()` without kernel-side plugin discovery or runtime-side
   module registration.

## Boundaries To Preserve

- Keep user-facing error mapping in `atom-ffi`.
- Keep validation close to the loader that owns the data.
- Keep codegen deterministic. Two identical inputs should produce the same plan and output tree.
- Keep generic backend layers backend-neutral. `atom-backends`, `atom-cng`, and `atom-deploy` must
  not encode concrete first-party backend ids, iOS/Android-specific branching, or backend- specific
  golden tests.
- Keep platform-specific environment diagnostics in `atom-backend-*` crates. `atom-cli` may
  aggregate `atom doctor` output, but it must not re-embed concrete iOS/Android probing logic.
- Keep disabled-backend failures side-effect free. CLI preflight for
  `atom run --platform <platform>` must reject disabled backends before CNG writes generated files.
- Keep backend-specific assertions and fixtures in `atom-backend-*` crates or schema-owning crates.
- Keep examples representative. The hello-world example should exercise real repo conventions, not a
  toy path that bypasses them.
- Keep first-party runtime libraries outside `atom-runtime`. The kernel owns singleton lifecycle and
  state primitives, while higher-level concerns like navigation and analytics stay in separate
  crates.

## When Adding A New Layer

- Document the responsibility here.
- Document any new invariants in [core-beliefs.md](core-beliefs.md) or a design doc if the change is
  architectural.
- Add verification coverage in [`../scripts/verify.sh`](../scripts/verify.sh) if the new layer
  changes repo-wide expectations.
