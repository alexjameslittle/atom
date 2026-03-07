# Atom Framework Specification

Status: Draft v1 (Rust-first, headless-mobile)

Purpose: Define a framework for building iOS and Android applications in Rust, with continuous native generation (CNG), generated native hosts, and Rust-authored native modules.

## 1. Problem Statement

Mobile teams that want a single application language usually end up choosing between:
- Native apps with split Swift/Kotlin codebases.
- Cross-platform runtimes with a second-language bridge.
- UI toolkits that own rendering but not native project generation.

Atom is intended to solve a different problem:
- Application logic should be authored in Rust.
- The framework should be headless first, so runtime, effects, module access, and lifecycle are shared across platforms.
- Native iOS and Android hosts should be generated continuously from configuration and module metadata.
- Native platform code should exist only as thin, framework-owned glue where possible.

Important boundary:
- Atom is a framework and generation system, not a hand-edited native template repo.
- Generated native output is framework-owned.
- A renderer may be added later, but it is not required to define the core framework contract.

## 2. Goals and Non-Goals

### 2.1 Goals

- Let application authors write app code in Rust.
- Define a stable app manifest contract in `Atom.toml`.
- Support Rust-authored native modules with typed Rust APIs.
- Generate iOS and Android host projects deterministically from app and module metadata.
- Use Bazel with `bzlmod` as the primary build system.
- Support reproducible local setup through `mise.toml`.
- Keep the framework bootstrap small enough to validate before attempting a renderer.

### 2.2 Non-Goals

- A full cross-platform widget system in v1.
- Manual edits inside generated host directories.
- Multiple build systems in parallel for the same source of truth.
- Web or desktop targets before iOS and Android are proven.
- Reproducing every Expo capability in the first implementation.

## 3. System Overview

### 3.1 Main Components

1. `Manifest Loader`
   - Reads `Atom.toml`.
   - Validates app and platform config.
   - Produces a typed app definition.
2. `App Runtime`
   - Owns lifecycle, state transitions, effects, and module access.
   - Runs as shared platform-agnostic Rust.
3. `Module SDK`
   - Defines how Rust modules expose app-facing APIs and generation metadata.
4. `CNG Engine`
   - Merges app config and module manifests into a platform generation plan.
   - Emits deterministic iOS and Android host trees.
5. `Bridge Layer`
   - Defines the stable ABI between Rust and generated Swift/Kotlin shims.
6. `CLI`
   - Exposes developer workflows such as `prebuild`, `run`, and `test`.
7. `Build Layer`
   - Uses Bazel for build, generation, and test execution.

### 3.2 Abstraction Levels

Atom should stay easy to evolve by keeping these layers distinct:

1. `Specification Layer`
   - `SPEC.md`
   - Defines contracts and behavior.
2. `Configuration Layer`
   - `Atom.toml`
   - App and platform settings.
3. `Runtime Layer`
   - Shared Rust lifecycle, state, effect handling, and module dispatch.
4. `Generation Layer`
   - CNG planning and native host generation.
5. `Integration Layer`
   - Swift/Kotlin shims, platform manifests, permissions, entitlements.
6. `Build Layer`
   - Bazel, `rules_rust`, and platform rules.

### 3.3 External Dependencies

- Bazel `8.4.2`
- `bazelisk`
- Rust toolchain `1.92.0`
- Xcode and Apple SDKs for iOS builds
- Android SDK/NDK for Android builds
- Bazel rules compatible with the target platform toolchains

## 4. Core Domain Model

### 4.1 Entities

#### 4.1.1 App Definition

The normalized app definition used by CNG and the runtime.

Fields:
- `name` (string)
- `slug` (string)
- `bundle_id_ios` (string)
- `application_id_android` (string)
- `entry_crate` (string)
- `modules` (list of module identifiers)
- `assets` (list)
- `build_profiles` (map)

#### 4.1.2 Module Definition

Normalized description of one Rust native module.

Fields:
- `id` (string)
- `crate_name` (string)
- `rust_api` (typed Rust surface)
- `permissions` (platform-specific list)
- `plist_fragments` (list)
- `android_manifest_fragments` (list)
- `entitlements` (list)
- `generated_sources` (list of source descriptors)
- `init_hooks` (list)

#### 4.1.3 Generation Plan

Platform-resolved output of CNG for one app.

Fields:
- `app` (App Definition)
- `ios_plan` (optional)
- `android_plan` (optional)
- `module_bindings` (list)
- `generated_files` (list)
- `warnings` (list)

#### 4.1.4 Platform Host

Generated native host tree for one platform.

Fields:
- `platform` (`ios` or `android`)
- `root` (path)
- `bootstrap_sources` (list)
- `config_files` (list)
- `asset_links` (list)
- `build_targets` (list)

### 4.2 Stable Identifier Rules

- `App Slug`
  - Lowercase, URL-safe, used in generated path names.
- `Module ID`
  - Globally unique within an app.
- `Generated Path`
  - Must be deterministic from app config, module graph, and generation version.
- `Platform Target Name`
  - Must remain stable across identical runs to avoid unnecessary build churn.

## 5. App Manifest Specification

### 5.1 File Discovery

Manifest file path precedence:
1. Explicit CLI path.
2. Default `Atom.toml` in the current repository root.

If the manifest cannot be read, generation fails.

### 5.2 File Format

`Atom.toml` is the repository-owned source of truth for app and platform configuration.

Expected top-level sections:
- `[app]`
- `[ios]`
- `[android]`
- `[build]`
- `[[modules]]`

Unknown keys should be rejected in strict mode and surfaced as warnings in permissive mode.

### 5.3 Minimum Schema

Required fields for `[app]`:
- `name`
- `slug`
- `entry_crate`

Required fields for `[ios]` when iOS generation is enabled:
- `bundle_id`
- `deployment_target`

Required fields for `[android]` when Android generation is enabled:
- `application_id`
- `min_sdk`
- `target_sdk`

Required fields for `[[modules]]`:
- `id`
- `crate`

### 5.4 Example Shape

```toml
[app]
name = "Hello Atom"
slug = "hello-atom"
entry_crate = "apps/hello_atom"

[ios]
bundle_id = "build.atom.hello"
deployment_target = "17.0"

[android]
application_id = "build.atom.hello"
min_sdk = 28
target_sdk = 35

[[modules]]
id = "device_info"
crate = "modules/device_info"
```

## 6. Module Specification

### 6.1 Module Contract

A module implementation must provide:
- A Rust-facing API used by the app.
- A manifest describing generation requirements.
- A platform bridge implementation when native interop is required.

### 6.2 Module Manifest Requirements

Each module manifest must be able to express:
- Required permissions
- `Info.plist` additions
- Android manifest additions
- Entitlements
- Startup or registration hooks
- Generated source templates or codegen descriptors

### 6.3 Ownership Rules

- App authors should consume modules from Rust.
- Swift/Kotlin written by module authors should be minimized and isolated.
- Framework-generated code should assemble the host registration table from module metadata.

## 7. Runtime Specification

### 7.1 Responsibilities

The shared runtime owns:
- App startup and shutdown
- State transitions
- Effect execution
- Navigation state
- Calls into native modules
- Structured logging hooks

### 7.2 Boundaries

- Platform-specific UI rendering is out of scope for the initial runtime contract.
- The runtime must not directly depend on iOS or Android SDK code.
- Platform access must cross the bridge layer through stable host interfaces.

## 8. Bridge Specification

### 8.1 Direction

The bridge should be framework-owned and optimized for generated hosts rather than designed as a general-purpose FFI product.

### 8.2 Requirements

- Stable bootstrap entrypoints from native hosts into Rust
- Deterministic module registration
- Async request/response support
- Event callback support
- Well-defined error transport across the boundary

### 8.3 Current Decision

The preferred direction is a custom C ABI for the framework boundary. This keeps lifecycle and registry behavior explicit and makes generated Swift/Kotlin glue straightforward. Other binding approaches can be evaluated later for module ergonomics, but they are not the primary contract.

## 9. Continuous Native Generation (CNG) Specification

### 9.1 Inputs

CNG consumes:
- `Atom.toml`
- App crate metadata
- Module manifests
- Build profile and environment inputs

### 9.2 Pipeline

1. Parse `Atom.toml`.
2. Resolve app and module graph.
3. Validate platform requirements.
4. Merge module requirements into platform plans.
5. Emit deterministic generated host trees.
6. Emit or update build metadata used by Bazel.

### 9.3 Outputs

CNG must be able to generate:
- `generated/ios/...`
- `generated/android/...`
- Platform config files
- Swift/Kotlin bootstrap and registration sources
- Bazel-readable generated source trees

### 9.4 Idempotence

Two identical CNG runs must produce byte-identical output, excluding timestamps or other explicitly ignored metadata.

### 9.5 Ownership

Anything under generated host roots is framework-owned. User customization should flow through manifest fields, config plugins, or module metadata rather than direct edits.

## 10. Build System Specification

### 10.1 Bazel

The repository must use:
- `bzlmod`
- `.bazelversion = 8.4.2`
- `MODULE.bazel` as the dependency entrypoint

### 10.2 Rust Setup

The Rust dependency model should follow the same pattern as the Forge repository:
- `rules_rust`
- `rust.toolchain(edition = "2024", versions = ["1.92.0"])`
- `crate_universe`
- One virtual dependency package under `third-party/rust`

Expected root layout:

```text
third-party/rust/
  BUILD.bazel
  Cargo.toml
  Cargo.lock
```

### 10.3 mise

`mise.toml` should pin:
- `bazel = "8.4.2"`
- `bazelisk`
- `rust = "1.92.0"`

### 10.4 Planned Platform Targets

Initial host and mobile coverage should include:
- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `aarch64-apple-ios`
- `aarch64-apple-ios-sim`
- `x86_64-apple-ios`
- `aarch64-linux-android`
- `x86_64-linux-android`

Additional targets may be added later when there is a clear support requirement.

## 11. CLI Specification

Minimum commands:
- `atom prebuild`
- `atom prebuild --dry-run`
- `atom run ios`
- `atom run android`
- `atom test`

`prebuild --dry-run` is the first command that should exist in the implementation. It provides the fastest validation that the manifest, module graph, and CNG planner are behaving correctly.

## 12. Repository Layout Specification

Expected initial layout:

```text
.
├── SPEC.md
├── .bazelversion
├── MODULE.bazel
├── mise.toml
├── docs/
│   └── plan.md
├── third-party/
│   └── rust/
├── crates/
│   ├── atom-runtime/
│   ├── atom-manifest/
│   ├── atom-modules/
│   ├── atom-ffi/
│   ├── atom-cng/
│   └── atom-cli/
├── templates/
│   ├── ios/
│   └── android/
└── examples/
```

This layout is a repository contract, not a claim that all directories exist yet.

## 13. Conformance Profiles

### 13.1 Phase 0: Toolchain Bootstrap

An implementation conforms to Phase 0 when it provides:
- `mise.toml`
- `.bazelversion`
- `MODULE.bazel`
- `third-party/rust`
- At least one Rust target building under Bazel

### 13.2 Phase 1: Manifest + Dry-Run CNG

An implementation conforms to Phase 1 when it provides:
- `Atom.toml` parsing and validation
- A typed generation plan
- `atom prebuild --dry-run`

### 13.3 Phase 2: Bootable Hosts

An implementation conforms to Phase 2 when it provides:
- Generated iOS host bootstrap
- Generated Android host bootstrap
- Rust app startup on both platforms

### 13.4 Phase 3: Core Runtime

An implementation conforms to Phase 3 when it provides:
- State management
- Effect execution
- Module invocation
- Structured logging

### 13.5 Phase 4: Developer Workflow

An implementation conforms to Phase 4 when it provides:
- `atom run ios`
- `atom run android`
- `atom test`
- A documented customization path that does not require manual edits to generated hosts

### 13.6 Phase 5: Optional Renderer

Renderer work is explicitly outside the minimum framework conformance profile. It can be specified later as an additive layer.

## 14. Open Questions

- Should generated Xcode projects be emitted directly, or derived later from Bazel?
- Should the Rust output be `staticlib`, `cdylib`, or both?
- Should Android-on-Linux support be a first-class target in the initial implementation or follow after macOS-first bring-up?
- How much module metadata should live in Rust macros versus `Atom.toml`?
- When a renderer is introduced, should it be specified in this document or a separate renderer spec?
