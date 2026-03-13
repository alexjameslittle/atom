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

`atom-runtime` and runtime plugin libraries remain separate from CLI/CNG graph orchestration.

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
- `atom-backends`
  - Owns backend contracts, shared destination/evaluation/CNG data types, and generic registries
  - Defines separate deploy-session seams for backend-owned debug-session orchestration and
    backend-owned UI automation
  - Defines the compile-time seam for first-party backend composition without dynamic loading
  - Stays platform-neutral; concrete iOS/Android behavior lives in backend implementation crates
- `atom-cng`
  - Validates framework-wide extension compatibility and delegates backend-specific compatibility
    checks to registered backends
  - Merges app + module + config-plugin configuration into deterministic generation plans
  - Dispatches backend planning and emission through registered `GenerationBackend` contracts
  - Writes generic schema/contributed files plus delegates backend host-tree emission
- `atom-cng-app-icon`
  - First-party config/CNG plugin crate implementing the public `ConfigPlugin` trait
  - Owns app icon config parsing, validation, file contributions, and platform resource wiring
- `atom-deploy`
  - Device discovery and destination selection
  - Dispatches run/stop/evaluate flows through registered `DeployBackend` contracts
  - Coordinates generic evaluation steps over backend-provided debug-session and UI-automation
    contracts without owning concrete debugger transport logic
  - Owns generic evidence capture, UI evaluation, and proof-bundle orchestration
  - Preserves compatibility fields such as serialized destination `platform`; backend ids are
    additive dispatch data, not replacements for stable machine-readable payload fields
  - Keeps platform deployment orchestration out of `atom-cli`
- `atom-backend-ios`
  - First-party iOS backend implementation crate
  - Registers iOS deploy and CNG backends for the canonical CLI binary
  - Owns iOS destination discovery, deploy/stop/evaluate implementation, and iOS host templates
  - Owns iOS-specific debug-session orchestration such as LLDB-facing launch or attach behavior
  - Owns iOS-specific CNG planning/emission and backend compatibility checks
- `atom-backend-android`
  - First-party Android backend implementation crate
  - Registers Android deploy and CNG backends for the canonical CLI binary
  - Owns Android destination discovery, deploy/stop/evaluate implementation, and Android host
    templates
  - Owns Android-specific debug-session orchestration such as JVM and native LLDB attach behavior
  - Owns Android-specific CNG planning/emission and backend compatibility checks
- `atom-cli`
  - Maps user commands to Bazel-aware workflows
  - Links the first-party config plugin registry used during `atom prebuild`
  - Builds the canonical first-party backend registries used for deploy/evaluate/CNG composition
  - Exposes uniform backend-aware verbs such as `atom run --platform <platform>`
  - Must stay a thin wrapper, not an alternate build system
- `atom-runtime`
  - Runtime kernel: lifecycle state machine (Created → Initializing → Running →
    Backgrounded/Suspended → Terminating → Terminated)
  - Kernel-owned state container and inspection snapshot (`RuntimeSnapshot`) for state values,
    dispatched events, completed effects, and module call records
  - Module lifecycle management: init in dependency order, shutdown in reverse
  - Runtime plugin host API (`RuntimePlugin` trait) for observing lifecycle events and owning
    plugin-local state
  - App-owned runtime config assembly through `atom_runtime_config()` in the app crate
  - Tokio `current_thread` async runtime available via `PluginContext`
  - `PluginContext` APIs for shared state writes, event/effect recording, async task execution, and
    runtime module calls
  - Structured logging via `tracing` at lifecycle transitions
  - Handle-based registry for FFI access from generated native hosts
  - `ensure_running()` gate plus runtime-side module method registration/call plumbing for
    Rust-backed modules
- `atom-navigation`
  - First-party runtime plugin crate that owns a route stack through the public `RuntimePlugin`
    contract
  - Proves navigation is a library concern rather than kernel state
- `atom-analytics`
  - First-party runtime plugin crate that buffers and flushes analytics batches on lifecycle
    boundaries
  - Proves non-routing headless app behavior composes through the same public plugin API

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
9. Generated runtime bridge code links the app crate and calls `atom_runtime_config()` without
   kernel-side plugin discovery. Any first-party or third-party plugin crates, along with any
   Rust-backed runtime module registrations, enter through that app-owned configuration path.

## Boundaries To Preserve

- Keep user-facing error mapping in `atom-ffi`.
- Keep validation close to the loader that owns the data.
- Keep codegen deterministic. Two identical inputs should produce the same plan and output tree.
- Keep generic backend layers backend-neutral. `atom-backends`, `atom-cng`, and `atom-deploy` must
  not encode concrete first-party backend ids, iOS/Android-specific branching, or backend- specific
  golden tests.
- Keep disabled-backend failures side-effect free. CLI preflight for
  `atom run --platform <platform>` must reject disabled backends before CNG writes generated files.
- Keep backend-specific assertions and fixtures in `atom-backend-*` crates or schema-owning crates.
- Keep examples representative. The hello-world example should exercise real repo conventions, not a
  toy path that bypasses them.
- Keep first-party plugins outside `atom-runtime`. The kernel owns lifecycle and registration, while
  higher-level concerns like navigation and analytics stay in separate crates.

## When Adding A New Layer

- Document the responsibility here.
- Document any new invariants in [core-beliefs.md](core-beliefs.md) or a design doc if the change is
  architectural.
- Add verification coverage in [`../scripts/verify.sh`](../scripts/verify.sh) if the new layer
  changes repo-wide expectations.
