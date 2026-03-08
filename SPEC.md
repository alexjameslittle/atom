# Atom Framework Specification

Status: Draft v2 (normative, Rust-first, headless-mobile)

This document is the source of truth for Atom's behavior. It is written so an implementation can be
derived from the spec without inventing missing behavior.

Normative language:

- `MUST`, `MUST NOT`, `REQUIRED`: hard requirements
- `SHOULD`, `SHOULD NOT`: strong defaults that may be relaxed with a clear reason
- `MAY`: optional behavior

## 1. Problem Statement

Atom defines a way to build iOS and Android applications where:

- application logic is authored in Rust
- the shared runtime is headless and platform-agnostic
- native host projects are continuously generated from config and module metadata
- native platform code is minimized to generated Swift/Kotlin glue

Atom is not:

- a hand-edited native template repository
- a JavaScript bridge
- a UI renderer in its minimum conformance profile

## 2. Goals and Non-Goals

### 2.1 Goals

- Define a stable Bazel-native app metadata model through `atom_app(...)`.
- Define a Rust module format that is consumable by both the runtime and CNG.
- Define deterministic CNG behavior and concrete generated outputs.
- Define a Bazel-first build contract using `bzlmod`.
- Define a small CLI with machine-verifiable behavior.
- Keep the first implementation slice narrow enough to validate quickly.

### 2.2 Non-Goals

- A full renderer in Phase 0 or Phase 1.
- Manual edits inside generated host directories.
- Dual build systems for the same source of truth.
- Desktop or web before mobile foundations are working.

## 3. Repository and Build Contract

The repository root MUST eventually contain:

```text
.
├── SPEC.md
├── .bazelversion
├── MODULE.bazel
├── mise.toml
├── bzl/
│   └── atom/
│       ├── defs.bzl
│       ├── atom_app.bzl
│       └── atom_module.bzl
├── docs/
│   └── plan.md
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

This layout is a contract for how the implementation is organized. It is not a claim that all
directories already exist.

Consumer model rules:

- Atom MUST be consumed as a Bazel module and ruleset.
- Consumer repositories MUST use Bazel and `bzlmod`.
- There is no supported Cargo-only, Xcode-only, or Gradle-only consumption model for Atom apps or
  modules.
- Xcode and Android Studio artifacts MAY exist only as derived outputs from the Bazel source of
  truth.
- The Atom CLI MUST act as a thin Bazel wrapper, not as an alternative build system.

Required build-system rules:

- Bazel is the primary build system.
- `WORKSPACE` MUST NOT be introduced.
- `.bazelversion` MUST be `8.4.2`.
- `mise.toml` MUST pin `bazel`, `bazelisk`, and `rust`.
- `MODULE.bazel` MUST use `bzlmod`.
- The Rust dependency model MUST be Bazel-native. Dependencies MUST be declared in `MODULE.bazel`
  via pinned `crate.spec(...)` entries and resolved through `crate_universe`.
- Repo-local `Cargo.toml` and `Cargo.lock` files MUST NOT be required for framework or consumer
  builds.
- The build layer MUST include the FlatBuffers compiler and the Rust `flatbuffers` crate.

Required pinned Bazel modules for the initial implementation:

- `bazel_skylib = 1.9.0`
- `platforms = 1.0.0`
- `rules_rust = 0.68.1`
- `flatbuffers = 25.9.23`

Required Rust toolchain defaults:

- edition `2024`
- version `1.92.0`

Required target triples for the first mobile-capable implementation:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `aarch64-apple-ios`
- `aarch64-apple-ios-sim`
- `x86_64-apple-ios`
- `aarch64-linux-android`
- `x86_64-linux-android`

Planned public Bazel surface:

- `atom_app` for consumer applications
- `atom_module` for Rust-authored Atom modules
- `atom_native_module` for native-only or mixed native modules
- `atom_schema_bundle` for module-owned FlatBuffers schemas

## 4. Error Taxonomy

Every user-facing failure MUST map to one of the following codes.

| Domain   | Code                         | Meaning                                          | CLI Exit |
| -------- | ---------------------------- | ------------------------------------------------ | -------- |
| Manifest | `MANIFEST_NOT_FOUND`         | generated app metadata could not be found        | `65`     |
| Manifest | `MANIFEST_PARSE_ERROR`       | generated app metadata could not be parsed       | `65`     |
| Manifest | `MANIFEST_MISSING_FIELD`     | required field missing                           | `65`     |
| Manifest | `MANIFEST_INVALID_VALUE`     | field type or value invalid                      | `65`     |
| Manifest | `MANIFEST_UNKNOWN_KEY`       | unknown field encountered                        | `65`     |
| Modules  | `MODULE_NOT_FOUND`           | configured module crate path missing             | `66`     |
| Modules  | `MODULE_DUPLICATE_ID`        | duplicate module identifier                      | `66`     |
| Modules  | `MODULE_DEPENDENCY_CYCLE`    | module dependency cycle detected                 | `66`     |
| Modules  | `MODULE_MANIFEST_INVALID`    | module manifest could not be loaded or validated | `66`     |
| CNG      | `CNG_CONFLICT`               | merge conflict with no legal resolution          | `67`     |
| CNG      | `CNG_TEMPLATE_ERROR`         | template or codegen failure                      | `67`     |
| CNG      | `CNG_WRITE_ERROR`            | generated files could not be written             | `67`     |
| Bridge   | `BRIDGE_INVALID_ARGUMENT`    | native host passed invalid ABI data              | `68`     |
| Bridge   | `BRIDGE_INIT_FAILED`         | runtime bridge bootstrap failed                  | `68`     |
| Runtime  | `RUNTIME_TRANSITION_INVALID` | invalid lifecycle transition                     | `68`     |
| Runtime  | `MODULE_INIT_FAILED`         | module init or shutdown hook failed              | `68`     |
| CLI      | `CLI_USAGE_ERROR`            | invalid CLI invocation                           | `64`     |
| Tooling  | `EXTERNAL_TOOL_FAILED`       | Bazel or another required tool failed            | `69`     |
| Internal | `INTERNAL_BUG`               | unexpected framework bug or invariant break      | `70`     |

Canonical machine-readable error payload:

```fbs
namespace atom.error;

table AtomError {
  code: string;
  message: string;
  path: string;
}
```

Rules:

- `AtomError.code` MUST match one of the taxonomy codes in this section.
- `path` SHOULD be present for manifest and module validation errors.
- `message` MUST be human-readable.
- Machine-readable outputs MUST emit exactly one `atom.error.AtomError` FlatBuffer on failure.

## 5. App Metadata Specification

### 5.1 File Discovery

App lookup order:

1. explicit `--target <label>`
2. fail with `CLI_USAGE_ERROR`

If the requested metadata target cannot be built or resolved, the command MUST fail with
`MANIFEST_NOT_FOUND`.

### 5.2 Format

`atom_app(...)` MUST be the source of truth for app configuration.

The Bazel rule MUST emit a single JSON metadata document with these top-level keys:

- `kind`
- `target_label`
- `name`
- `slug`
- `entry_crate_label`
- `generated_root`
- `watch`
- `ios`
- `android`
- `modules`

Unknown keys MUST fail validation with `MANIFEST_UNKNOWN_KEY`.

### 5.3 Field Cheat Sheet

| Key                 | Type          | Required         | Default                   | Validation                          |
| ------------------- | ------------- | ---------------- | ------------------------- | ----------------------------------- |
| `name`              | string        | yes              | none                      | non-empty UTF-8                     |
| `slug`              | string        | yes              | none                      | regex `^[a-z][a-z0-9-]{1,62}$`      |
| `entry_crate_label` | string        | yes              | none                      | absolute Bazel label                |
| `generated_root`    | string        | no               | `"generated"`             | relative path, MUST NOT be absolute |
| `watch`             | bool          | no               | `false`                   | boolean                             |
| `ios.enabled`       | bool          | no               | `true` if section present | boolean                             |
| `bundle_id`         | string        | yes when enabled | none                      | reverse-DNS identifier              |
| `deployment_target` | string        | yes when enabled | none                      | regex `^[0-9]+\\.[0-9]+$`           |
| `android.enabled`   | bool          | no               | `true` if section present | boolean                             |
| `application_id`    | string        | yes when enabled | none                      | reverse-DNS identifier              |
| `min_sdk`           | integer       | yes when enabled | none                      | `>= 24`                             |
| `target_sdk`        | integer       | yes when enabled | none                      | `>= min_sdk`                        |
| `modules`           | array<string> | no               | `[]`                      | absolute Bazel labels, unique       |

### 5.4 Validation Rules

- At least one platform section MUST be enabled.
- `app.slug` MUST be unique within generated output paths.
- `android.target_sdk` MUST be greater than or equal to `android.min_sdk`.
- Module target labels MUST be unique across `modules`.
- `generated_root` MUST be relative to the repo root.

### 5.5 Canonical Example

```json
{
  "kind": "atom_app",
  "target_label": "//apps/hello_atom:hello_atom",
  "name": "Hello Atom",
  "slug": "hello-atom",
  "entry_crate_label": "//apps/hello_atom:hello_atom",
  "generated_root": "generated",
  "watch": false,
  "ios": {
    "enabled": true,
    "bundle_id": "build.atom.hello",
    "deployment_target": "17.0"
  },
  "android": {
    "enabled": true,
    "application_id": "build.atom.hello",
    "min_sdk": 28,
    "target_sdk": 35
  },
  "modules": ["//modules/device_info:device_info"]
}
```

### 5.6 Reference Algorithm: Metadata Loading

```text
function load_manifest(repo_root, app_target):
    metadata_target = derive_metadata_target(app_target, "_atom_app_metadata")
    metadata_path = bazel_build_and_locate(repo_root, metadata_target)
    if metadata_path does not exist:
        error MANIFEST_NOT_FOUND at metadata_target

    raw = read_text(metadata_path) or error MANIFEST_PARSE_ERROR
    parsed = parse_json(raw) or error MANIFEST_PARSE_ERROR

    reject_unknown_top_level_keys(parsed)
    validate parsed["kind"] == "atom_app"
    app = validate_app_metadata(parsed)
    ios = validate_ios_section(parsed.get("ios"))
    android = validate_android_section(parsed.get("android"))
    build = validate_build_section(parsed.get("generated_root"), parsed.get("watch"))
    modules = validate_module_array(parsed.get("modules", []))

    if not ios.enabled and not android.enabled:
        error MANIFEST_INVALID_VALUE at "ios/android"

    return NormalizedManifest(app, ios, android, build, modules)
```

## 6. Module Specification

### 6.1 Source Format

Each module crate MUST define exactly one Rust type annotated with `#[atom_module]` and implementing
`AtomModule`.

Required source shape:

```rust
#[atom_module]
pub struct DeviceInfoModule;

impl AtomModule for DeviceInfoModule {
    fn manifest() -> ModuleManifest {
        ModuleManifest::new("device_info")
            .schema_file("schema/device_info.fbs")
    }

    fn exports(exports: &mut ModuleExports) {
        exports.export::<device_info_fb::GetDeviceInfoRequest, device_info_fb::GetDeviceInfoResponse>(
            "get",
            device_info_get,
        );
    }
}
```

### 6.2 Required Trait Surface

```rust
pub trait AtomModule {
    fn manifest() -> ModuleManifest;
    fn exports(exports: &mut ModuleExports);
}
```

`ModuleManifest` MUST support these fields:

- `id: String`
- `depends_on: Vec<String>`
- `schema_files: Vec<String>`
- `methods: Vec<MethodSpec>`
- `permissions: Vec<PermissionSpec>`
- `plist: JsonMap`
- `android_manifest: JsonMap`
- `entitlements: JsonMap`
- `generated_sources: Vec<GeneratedSourceSpec>`
- `init_priority: i32`

`MethodSpec` MUST support these fields:

- `name: String`
- `request_table: String`
- `response_table: String`

Schema source of truth rules:

- `.fbs` files are the only source of truth for the wire contract.
- Each module MUST declare one or more FlatBuffers schema files in `schema_files`.
- Schema file paths MUST be relative to the module crate root.
- Existing FlatBuffers schemas MAY be reused unchanged by listing them in `schema_files`.
- The proc macro MUST NOT infer FlatBuffers table definitions from Rust struct fields.
- Rust request and response types used at the ABI boundary MUST be generated from `.fbs`.
- Handwritten Rust structs and enums MAY exist as implementation details, but they MUST NOT define
  or evolve the wire contract.
- `MethodSpec.request_table` and `MethodSpec.response_table` MUST be fully qualified FlatBuffers
  table names declared by the module's schema files.

### 6.3 Build-Time Manifest Extraction

The `#[atom_module]` macro MUST generate a manifest export symbol:

```c
AtomOwnedBuffer atom_module_manifest_flatbuffer(void);
```

Meaning:

- it returns canonical FlatBuffer metadata for the module manifest
- the returned buffer is allocated by Rust
- callers MUST free it with `atom_buffer_free`

CNG MUST resolve module manifests by invoking this generated export through a framework helper
binary. The helper binary is part of the implementation, but the externally visible contract is the
FlatBuffer payload.

The manifest payload MUST include:

- `schema_files`
- fully qualified request and response table names for every method
- enough metadata for CNG to assemble an aggregate schema and drive language binding generation

CNG MUST treat the module-owned `.fbs` files as the source of truth. It MUST NOT synthesize
FlatBuffers table definitions from Rust structs.

### 6.4 Module Resolution Rules

- Requested modules are taken from `[[modules]]` in manifest declaration order.
- A module dependency graph is formed using `depends_on`.
- Resolution order MUST be a topological sort of dependencies.
- For ties, declaration order MUST win.
- Duplicate IDs MUST fail with `MODULE_DUPLICATE_ID`.
- Dependency cycles MUST fail with `MODULE_DEPENDENCY_CYCLE`.

### 6.5 Reference Algorithm: Module Resolution

```text
function resolve_modules(requested_modules):
    loaded = []
    seen_ids = set()

    for request in requested_modules in declaration order:
        if request.id in seen_ids:
            error MODULE_DUPLICATE_ID at "modules." + request.id
        if crate path does not exist:
            error MODULE_NOT_FOUND at request.crate

        manifest_flatbuffer = extract_module_manifest_flatbuffer(request.crate)
        manifest = parse_flatbuffer(manifest_flatbuffer) or error MODULE_MANIFEST_INVALID
        validate manifest.id == request.id or error MODULE_MANIFEST_INVALID

        loaded.push((request, manifest))
        seen_ids.add(request.id)

    graph = build_dependency_graph(loaded)
    ordered = topological_sort(graph) or error MODULE_DEPENDENCY_CYCLE
    return ordered using declaration order as tie-breaker
```

## 7. Runtime Specification

### 7.1 Runtime States

The runtime state machine MUST use these states:

- `Created`
- `Initializing`
- `Running`
- `Backgrounded`
- `Suspended`
- `Terminating`
- `Terminated`
- `Failed`

### 7.2 Transition Rules

| From                                       | Trigger                             | To             | Required Action                         |
| ------------------------------------------ | ----------------------------------- | -------------- | --------------------------------------- |
| `Created`                                  | `atom_app_init` succeeds            | `Initializing` | allocate runtime handle, parse config   |
| `Initializing`                             | all modules initialize successfully | `Running`      | mark runtime available for module calls |
| `Initializing`                             | module init fails                   | `Failed`       | emit `MODULE_INIT_FAILED`               |
| `Running`                                  | host foreground loss                | `Backgrounded` | pause foreground-only work              |
| `Backgrounded`                             | host foreground gain                | `Running`      | resume foreground-only work             |
| `Backgrounded`                             | host suspension callback            | `Suspended`    | flush state, stop non-background tasks  |
| `Suspended`                                | host resume callback                | `Running`      | restore active scheduling               |
| `Running` or `Backgrounded` or `Suspended` | host terminate callback             | `Terminating`  | begin reverse-order module shutdown     |
| `Terminating`                              | shutdown completes                  | `Terminated`   | free runtime resources                  |
| any non-terminal state                     | unrecoverable bridge/runtime error  | `Failed`       | freeze further transitions              |

### 7.3 Module Initialization Order

- Modules MUST initialize in resolved module order from Section 6.4.
- Shutdown MUST happen in reverse initialization order.
- Module `init_priority` MAY reorder modules within the same dependency layer.
- If two modules share the same dependency layer and priority, declaration order MUST win.

### 7.4 Invalid Transitions

These transitions MUST fail with `RUNTIME_TRANSITION_INVALID`:

- `Created -> Backgrounded`
- `Running -> Initializing`
- `Terminated -> any state`
- `Failed -> any state except external process restart`

### 7.5 Reference Algorithm: Runtime Startup

```text
function start_runtime(normalized_manifest, resolved_modules):
    state = Created
    handle = allocate_runtime_handle()
    state = Initializing

    ordered = sort_modules_for_init(resolved_modules)
    for module in ordered:
        result = module.init()
        if result is error:
            state = Failed
            error MODULE_INIT_FAILED at module.id

    state = Running
    return handle
```

## 8. Bridge ABI Specification

### 8.1 Primitive Types

```c
typedef struct {
  const uint8_t *ptr;
  uintptr_t len;
} AtomSlice;

typedef struct {
  uint8_t *ptr;
  uintptr_t len;
  uintptr_t cap;
} AtomOwnedBuffer;

typedef uint64_t AtomRuntimeHandle;

typedef enum {
  ATOM_LIFECYCLE_FOREGROUND = 1,
  ATOM_LIFECYCLE_BACKGROUND = 2,
  ATOM_LIFECYCLE_SUSPEND = 3,
  ATOM_LIFECYCLE_RESUME = 4,
  ATOM_LIFECYCLE_TERMINATE = 5
} AtomLifecycleEvent;
```

### 8.2 Required Functions

```c
int32_t atom_app_init(
  AtomSlice config_flatbuffer,
  AtomRuntimeHandle *out_handle,
  AtomOwnedBuffer *out_error_flatbuffer
);

int32_t atom_app_handle_lifecycle(
  AtomRuntimeHandle handle,
  AtomLifecycleEvent event,
  AtomOwnedBuffer *out_error_flatbuffer
);

void atom_app_shutdown(AtomRuntimeHandle handle);
void atom_buffer_free(AtomOwnedBuffer buffer);
```

Return rules:

- `0` means success
- non-zero means failure and MUST populate `out_error_flatbuffer`

### 8.3 Generated Method Exports

For each module method declared through `exports.export::<Request, Response>(...)`, CNG MUST
generate a direct Rust export with this shape:

```c
int32_t atom_device_info_get(
  AtomSlice input_flatbuffer,
  AtomOwnedBuffer *out_response_flatbuffer,
  AtomOwnedBuffer *out_error_flatbuffer
);
```

Export naming rules:

- generated export names MUST use the form `atom_<module_id>_<method_name>`
- `<module_id>` and `<method_name>` MUST use the normalized manifest identifiers
- export name collisions MUST fail generation with `CNG_CONFLICT`

### 8.4 Memory Ownership Rules

- `AtomSlice` is caller-owned and borrowed for the duration of the call only.
- `AtomOwnedBuffer` returned by Rust is Rust-owned until the caller frees it with
  `atom_buffer_free`.
- `config_flatbuffer`, `input_flatbuffer`, `out_response_flatbuffer`, and `out_error_flatbuffer`
  MUST conform to the generated FlatBuffers schema for the current app build.
- Hosts MUST NOT retain borrowed pointers after the call returns.

### 8.5 Wire Format

Runtime module calls MUST use CNG-generated FlatBuffers, not JSON.

CNG MUST emit:

- `generated/schema/atom.fbs`
- `generated/schema/modules/<module_id>/...` for each declared module schema file

The build layer MUST generate Rust bindings for the runtime side and Swift/Kotlin bindings for
native hosts from the aggregate schema plus all module-owned schema files.

Those generated bindings MUST define the Rust request and response types used with
`exports.export::<Request, Response>(...)`.

The `input_flatbuffer` payload MUST be the method-specific request table, not a generic wrapper
envelope.

`generated/schema/atom.fbs` MUST be an aggregate root file. It MUST include module-owned schemas
rather than rewriting them.

Canonical example:

```fbs
include "modules/device_info/device_info.fbs";

namespace atom;

table AtomAppConfig {
  name: string;
  slug: string;
}
```

Module-owned schema example:

```fbs
namespace atom.device_info;

table GetDeviceInfoRequest {}

table GetDeviceInfoResponse {
  model: string;
  os: string;
}
```

The generated Rust module API MAY remain fully typed and ergonomic. The FlatBuffer boundary exists
to keep the host/runtime transport compact and deterministic.

Successful responses from generated per-method exports MUST be method-specific FlatBuffer response
buffers.

Failures from `atom_app_init`, `atom_app_handle_lifecycle`, and generated per-method exports MUST
return an `atom.error.AtomError` FlatBuffer in `out_error_flatbuffer`.

The app bootstrap payload passed to `atom_app_init` MUST also be a CNG-generated FlatBuffer payload
that contains normalized app configuration and platform startup settings.

The initial bridge profile is synchronous only. Async module work MAY happen inside the Rust
runtime, but it MUST complete behind the synchronous generated method export boundary until a future
ABI revision adds explicit async transport.

## 9. Continuous Native Generation (CNG) Specification

### 9.1 Inputs

CNG consumes:

- normalized `atom_app(...)` metadata
- resolved module metadata
- module-owned FlatBuffers schema files
- selected platform set
- build profile

### 9.2 Merge Rules

| Artifact              | Rule                               | Conflict Behavior                                  |
| --------------------- | ---------------------------------- | -------------------------------------------------- |
| permissions           | set union, lexicographic sort      | never conflicts                                    |
| `plist` maps          | deep merge                         | conflicting scalar values fail with `CNG_CONFLICT` |
| Android manifest maps | deep merge                         | conflicting scalar values fail with `CNG_CONFLICT` |
| entitlements          | deep merge                         | conflicting scalar values fail with `CNG_CONFLICT` |
| generated sources     | concatenate in stable module order | never conflicts                                    |
| init hooks            | stable module order                | never conflicts                                    |

The app manifest MAY later add explicit override sections. Until then, conflicting scalar values
MUST fail generation.

### 9.3 Reference Algorithm: Plan Merge

```text
function build_generation_plan(manifest, modules):
    plan = new GenerationPlan()
    plan.app = manifest.app

    for module in modules in resolved order:
        plan.permissions = union(plan.permissions, module.permissions)
        plan.plist = deep_merge(plan.plist, module.plist) or error CNG_CONFLICT
        plan.android_manifest = deep_merge(plan.android_manifest, module.android_manifest) or error CNG_CONFLICT
        plan.entitlements = deep_merge(plan.entitlements, module.entitlements) or error CNG_CONFLICT
        plan.generated_sources.extend(module.generated_sources)
        plan.module_bindings.append(module.id)

    if manifest.ios.enabled:
        plan.ios = build_ios_plan(manifest, plan)
    if manifest.android.enabled:
        plan.android = build_android_plan(manifest, plan)

    return plan
```

### 9.4 Reference Algorithm: Host Tree Emission

```text
function emit_host_tree(plan):
    roots = []

    if plan.ios exists:
        root = generated_root / "ios" / plan.app.slug
        write_ios_files(root, plan)
        roots.push(root)

    if plan.android exists:
        root = generated_root / "android" / plan.app.slug
        write_android_files(root, plan)
        roots.push(root)

    return roots
```

### 9.5 Concrete Output Format

For the canonical `hello-atom` app with the `device_info` module, CNG MUST emit this file tree:

```text
generated/
├── schema/
│   ├── atom.fbs
│   └── modules/
│       └── device_info/
│           └── device_info.fbs
├── ios/
│   └── hello-atom/
│       ├── BUILD.bazel
│       ├── Info.generated.plist
│       ├── AtomAppDelegate.swift
│       ├── AtomBindings.swift
│       └── main.swift
└── android/
    └── hello-atom/
        ├── BUILD.bazel
        ├── AndroidManifest.generated.xml
        └── src/main/kotlin/build/atom/hello/
            ├── AtomApplication.kt
            ├── AtomBindings.kt
            └── MainActivity.kt
```

Rules:

- Bazel targets in generated roots MUST define an `:app` target.
- CNG MUST emit `generated/schema/atom.fbs`.
- CNG MUST preserve module-owned schema files under `generated/schema/modules/<module_id>/...`.
- iOS generated roots MUST contain `Info.generated.plist`.
- Android generated roots MUST contain `AndroidManifest.generated.xml`.
- Generated file names MUST be stable across identical runs.

### 9.6 Dry-Run Output Format

`atom prebuild --dry-run` MUST emit an `atom.cli.PrebuildPlan` FlatBuffer to stdout.

Canonical payload schema:

```fbs
namespace atom.cli;

table PrebuildApp {
  name: string;
  slug: string;
  entry_crate: string;
}

table PrebuildModule {
  id: string;
  init_order: uint32;
  crate: string;
}

table PrebuildPlatform {
  generated_root: string;
  target: string;
}

table PrebuildSchema {
  aggregate: string;
  modules: [string];
}

table PrebuildPlan {
  version: uint16;
  status: string;
  app: PrebuildApp;
  modules: [PrebuildModule];
  ios: PrebuildPlatform;
  android: PrebuildPlatform;
  schema: PrebuildSchema;
  generated_files: [string];
  warnings: [string];
}
```

For the canonical `hello-atom` example, the `atom.cli.PrebuildPlan` payload MUST contain:

- `app.name = "Hello Atom"`
- `app.slug = "hello-atom"`
- `app.entry_crate = "apps/hello_atom"`
- one module entry with `id = "device_info"` and `crate = "modules/device_info"`
- `ios.generated_root = "generated/ios/hello-atom"`
- `ios.target = "//generated/ios/hello-atom:app"`
- `android.generated_root = "generated/android/hello-atom"`
- `android.target = "//generated/android/hello-atom:app"`
- `schema.aggregate = "generated/schema/atom.fbs"`
- `schema.modules[0] = "generated/schema/modules/device_info/device_info.fbs"`
- `generated_files` containing, at minimum, `generated/schema/atom.fbs`,
  `generated/schema/modules/device_info/device_info.fbs`, `generated/ios/hello-atom/BUILD.bazel`,
  `generated/ios/hello-atom/Info.generated.plist`, `generated/ios/hello-atom/AtomAppDelegate.swift`,
  `generated/ios/hello-atom/AtomBindings.swift`, `generated/ios/hello-atom/main.swift`,
  `generated/android/hello-atom/BUILD.bazel`,
  `generated/android/hello-atom/AndroidManifest.generated.xml`,
  `generated/android/hello-atom/src/main/kotlin/build/atom/hello/AtomApplication.kt`,
  `generated/android/hello-atom/src/main/kotlin/build/atom/hello/AtomBindings.kt`, and
  `generated/android/hello-atom/src/main/kotlin/build/atom/hello/MainActivity.kt`

### 9.7 Watch Semantics

Watch mode is not required for Phase 1, but its behavior is defined now to avoid ambiguity later.

If `atom prebuild --watch` is implemented, it MUST:

- watch the app metadata target inputs
- watch configured module target inputs
- watch generated templates
- rerun manifest load, module resolution, and plan merge on each stable file event

Failure behavior in watch mode:

- the last successful generated output MUST remain on disk
- the failing iteration MUST report an error
- the watch process MUST continue running

## 10. CLI Specification

### 10.1 Commands

Required commands:

- `atom prebuild`
- `atom prebuild --dry-run`
- `atom run ios`
- `atom run android`
- `atom test`

### 10.2 Exit Codes

| Exit | Meaning                 |
| ---- | ----------------------- |
| `0`  | success                 |
| `64` | CLI usage error         |
| `65` | manifest error          |
| `66` | module resolution error |
| `67` | CNG error               |
| `68` | bridge or runtime error |
| `69` | external tool failure   |
| `70` | internal bug            |

### 10.3 Output Rules

All CLI commands MUST fail with `CLI_USAGE_ERROR` when invoked outside a Bazel workspace that
consumes Atom via `bzlmod`.

`atom prebuild --dry-run`:

- MUST write canonical `atom.cli.PrebuildPlan` FlatBuffer to stdout on success
- MUST NOT write generated files
- MUST write exactly one `atom.error.AtomError` FlatBuffer to stderr on failure

`atom prebuild`:

- MUST generate files under `build.generated_root`
- SHOULD write one summary line per generated platform root

`atom run ios`:

- MUST invoke `bazel run //generated/ios/<slug>:app`

`atom run android`:

- MUST invoke `bazel run //generated/android/<slug>:app`

`atom test`:

- MUST invoke `bazel test //...`

### 10.4 Reference Algorithm: `prebuild --dry-run`

```text
function cli_prebuild_dry_run(args):
    manifest = load_manifest(args.repo_root, args.manifest)
    modules = resolve_modules(manifest.modules)
    plan = build_generation_plan(manifest, modules)
    preview = render_generation_preview(plan)
    write_flatbuffer_stdout(preview)
    exit 0
```

## 11. Conformance Profiles

### 11.1 Phase 0: Toolchain Bootstrap

Required artifacts:

- `.bazelversion`
- `MODULE.bazel`
- `mise.toml`
- `bzl/atom/{defs.bzl,atom_app.bzl,atom_module.bzl}`
- at least one Rust target building under Bazel

Conformance example:

- Input: fresh repo with toolchain files
- Expected output: `bazel test //...` exits `0`

### 11.2 Phase 1: Manifest + Dry-Run CNG

Required behavior:

- `atom_app(...)` metadata parses and validates
- module metadata resolves
- `atom prebuild --dry-run` returns a canonical `atom.cli.PrebuildPlan` FlatBuffer

Conformance example:

- Input: the canonical example from Section 5.5
- Expected output: the canonical `atom.cli.PrebuildPlan` payload from Section 9.6

### 11.3 Phase 2: Bootable Hosts

Required behavior:

- iOS and Android host trees emit the file names from Section 9.5
- each generated root contains a Bazel `:app` target
- the Rust runtime can start from the native host

Conformance example:

- Input: canonical `hello-atom` app
- Expected output: file tree from Section 9.5 plus successful `atom run ios` and `atom run android`

### 11.4 Phase 3: Core Runtime

Required behavior:

- module init in resolved order
- runtime lifecycle follows Section 7.2
- invalid transitions return `RUNTIME_TRANSITION_INVALID`

Conformance example:

- Input sequence: `init -> background -> resume -> terminate`
- Expected state sequence:
  `Created -> Initializing -> Running -> Backgrounded -> Running -> Terminating -> Terminated`

### 11.5 Phase 4: Developer Workflow

Required behavior:

- CLI commands behave as defined in Section 10
- generated outputs remain framework-owned
- customization path exists without manual edits to generated roots

Conformance example:

- Input: `atom test`
- Expected output: wrapper around `bazel test //...` with matching exit code

### 11.6 Phase 5: Optional Renderer

Renderer behavior is intentionally outside the minimum Atom conformance profile and SHOULD be
specified in a separate renderer spec if and when that work begins.

## 12. Open Questions

- Should Xcode projects be emitted directly, or derived from Bazel later?
- Should the runtime artifact be `staticlib`, `cdylib`, or both?
- Should Android-on-Linux be first-class in the first host-capable milestone or follow macOS-first
  bring-up?
- Should app-level override sections be added to resolve plist and manifest merge conflicts?
- Should renderer work live in this spec or a dedicated additive spec?
