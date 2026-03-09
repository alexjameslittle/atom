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
- the runtime kernel provides one stable execution model for apps and runtime plugins
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
- Define a runtime plugin model that is distinct from native modules.
- Define a runtime plugin model that supports first-party and third-party library crates through the
  same public API.
- Define a config/CNG plugin model for deterministic native host customization.
- Define deterministic CNG behavior and concrete generated outputs.
- Define a Bazel-first build contract using `bzlmod`.
- Define a small CLI with machine-verifiable behavior.
- Define a framework-owned evaluation surface for logs, evidence capture, UI inspection, and basic
  UI interaction on runnable platform destinations.
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
- `rules_apple = 3.16.1`
- `rules_swift = 2.1.1`
- `apple_support = 1.24.2`
- `rules_kotlin = 1.9.0`
- `rules_java = 9.3.0`

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
- plugin-specific Starlark macros consumed through `atom_app(...).config_plugins`
- `atom_schema_bundle` for module-owned FlatBuffers schemas

## 4. Error Taxonomy

Every user-facing failure MUST map to one of the following codes.

| Domain    | Code                            | Meaning                                          | CLI Exit |
| --------- | ------------------------------- | ------------------------------------------------ | -------- |
| Manifest  | `MANIFEST_NOT_FOUND`            | generated app metadata could not be found        | `65`     |
| Manifest  | `MANIFEST_PARSE_ERROR`          | generated app metadata could not be parsed       | `65`     |
| Manifest  | `MANIFEST_MISSING_FIELD`        | required field missing                           | `65`     |
| Manifest  | `MANIFEST_INVALID_VALUE`        | field type or value invalid                      | `65`     |
| Manifest  | `MANIFEST_UNKNOWN_KEY`          | unknown field encountered                        | `65`     |
| Modules   | `MODULE_NOT_FOUND`              | configured module crate path missing             | `66`     |
| Modules   | `MODULE_DUPLICATE_ID`           | duplicate module identifier                      | `66`     |
| Modules   | `MODULE_DEPENDENCY_CYCLE`       | module dependency cycle detected                 | `66`     |
| Modules   | `MODULE_MANIFEST_INVALID`       | module manifest could not be loaded or validated | `66`     |
| Extension | `EXTENSION_INCOMPATIBLE`        | module or config plugin is incompatible          | `66`     |
| CNG       | `CNG_CONFLICT`                  | merge conflict with no legal resolution          | `67`     |
| CNG       | `CNG_TEMPLATE_ERROR`            | template or codegen failure                      | `67`     |
| CNG       | `CNG_WRITE_ERROR`               | generated files could not be written             | `67`     |
| Bridge    | `BRIDGE_INVALID_ARGUMENT`       | native host passed invalid ABI data              | `68`     |
| Bridge    | `BRIDGE_INIT_FAILED`            | runtime bridge bootstrap failed                  | `68`     |
| Runtime   | `RUNTIME_TRANSITION_INVALID`    | invalid lifecycle transition                     | `68`     |
| Runtime   | `MODULE_INIT_FAILED`            | module init or shutdown hook failed              | `68`     |
| CLI       | `CLI_USAGE_ERROR`               | invalid CLI invocation                           | `64`     |
| Auto      | `AUTOMATION_UNAVAILABLE`        | required automation backend unavailable          | `69`     |
| Auto      | `AUTOMATION_TARGET_NOT_FOUND`   | requested UI target could not be resolved        | `69`     |
| Auto      | `AUTOMATION_LOG_CAPTURE_FAILED` | requested logs could not be collected            | `69`     |
| Tooling   | `EXTERNAL_TOOL_FAILED`          | Bazel or another required tool failed            | `69`     |
| Internal  | `INTERNAL_BUG`                  | unexpected framework bug or invariant break      | `70`     |

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
- `path` SHOULD be present for manifest and extension validation errors.
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
- `entry_crate_name`
- `generated_root`
- `watch`
- `ios`
- `android`
- `modules`
- `config_plugins`

Unknown keys MUST fail validation with `MANIFEST_UNKNOWN_KEY`.

### 5.3 Field Cheat Sheet

| Key                 | Type          | Required         | Default                   | Validation                                       |
| ------------------- | ------------- | ---------------- | ------------------------- | ------------------------------------------------ |
| `name`              | string        | yes              | none                      | non-empty UTF-8                                  |
| `slug`              | string        | yes              | none                      | regex `^[a-z][a-z0-9-]{1,62}$`                   |
| `entry_crate_label` | string        | yes              | none                      | absolute Bazel label                             |
| `entry_crate_name`  | string        | yes              | none                      | regex `^[A-Za-z_][A-Za-z0-9_]*$`                 |
| `generated_root`    | string        | no               | `"generated"`             | relative path, MUST NOT be absolute              |
| `watch`             | bool          | no               | `false`                   | boolean                                          |
| `ios.enabled`       | bool          | no               | `true` if section present | boolean                                          |
| `bundle_id`         | string        | yes when enabled | none                      | reverse-DNS identifier                           |
| `deployment_target` | string        | yes when enabled | none                      | regex `^[0-9]+\\.[0-9]+$`                        |
| `android.enabled`   | bool          | no               | `true` if section present | boolean                                          |
| `application_id`    | string        | yes when enabled | none                      | reverse-DNS identifier                           |
| `min_sdk`           | integer       | yes when enabled | none                      | `>= 24`                                          |
| `target_sdk`        | integer       | yes when enabled | none                      | `>= min_sdk`                                     |
| `modules`           | array<string> | no               | `[]`                      | absolute Bazel labels, unique                    |
| `config_plugins`    | array<object> | no               | `[]`                      | entries require unique `id` plus object `config` |

Each `config_plugins` entry MUST support these fields:

- `id: String`
- `atom_api_level: u32`
- `min_atom_version: Option<String>`
- `ios_min_deployment_target: Option<String>`
- `android_min_sdk: Option<u32>`
- `config: JsonMap`

### 5.4 Validation Rules

- At least one platform section MUST be enabled.
- `app.slug` MUST be unique within generated output paths.
- `android.target_sdk` MUST be greater than or equal to `android.min_sdk`.
- Module target labels MUST be unique across `modules`.
- Config-plugin IDs MUST be unique across `config_plugins`.
- `generated_root` MUST be relative to the repo root.

### 5.5 Canonical Example

```json
{
  "kind": "atom_app",
  "target_label": "//apps/hello_atom:hello_atom",
  "name": "Hello Atom",
  "slug": "hello-atom",
  "entry_crate_label": "//apps/hello_atom:hello_atom",
  "entry_crate_name": "hello_atom",
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
  "modules": ["//modules/device_info:device_info"],
  "config_plugins": []
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
    config_plugins = validate_config_plugin_array(parsed.get("config_plugins", []))

    if not ios.enabled and not android.enabled:
        error MANIFEST_INVALID_VALUE at "ios/android"

    return NormalizedManifest(app, ios, android, build, modules, config_plugins)
```

## 6. Module Specification

### 6.1 Source Of Truth

`atom_module(...)` and `atom_native_module(...)` MUST be the source of truth for module metadata
consumed by module resolution and CNG.

Rules:

- requested modules are identified by Bazel target labels listed in `atom_app(...).modules`
- `atom_module(...)` is for Rust-authored modules that also compile a Rust library target
- `atom_native_module(...)` is for native-only or mixed native modules that do not require a Rust
  library target
- module discovery MUST occur by building and loading the generated `<target>_atom_module_metadata`
  JSON target emitted by the Bazel rule
- optional Rust helper traits or proc macros MAY exist for ergonomics, but module discovery and CNG
  MUST NOT depend on proc-macro-generated manifest exports or runtime reflection
- `.fbs` files remain the source of truth for ABI-visible request and response payloads

### 6.2 Required Metadata Shape

The Bazel-generated metadata document MUST include these top-level keys:

- `kind`
- `target_label`
- `id`
- `atom_api_level`
- `min_atom_version`
- `ios_min_deployment_target`
- `android_min_sdk`
- `depends_on`
- `schema_files`
- `methods`
- `permissions`
- `plist`
- `android_manifest`
- `entitlements`
- `generated_sources`
- `init_priority`
- `ios_srcs`
- `android_srcs`

Unknown keys MUST fail validation with `MODULE_MANIFEST_INVALID`.

The normalized module manifest MUST support these fields:

- `id: String`
- `atom_api_level: u32`
- `min_atom_version: Option<String>`
- `ios_min_deployment_target: Option<String>`
- `android_min_sdk: Option<u32>`
- `depends_on: Vec<String>`
- `schema_files: Vec<String>`
- `methods: Vec<MethodSpec>`
- `permissions: Vec<PermissionSpec>`
- `plist: JsonMap`
- `android_manifest: JsonMap`
- `entitlements: JsonMap`
- `generated_sources: Vec<GeneratedSourceSpec>`
- `init_priority: i32`
- `ios_srcs: Vec<String>`
- `android_srcs: Vec<String>`

`MethodSpec` MUST support these fields:

- `name: String`
- `request_table: String`
- `response_table: String`

Schema source of truth rules:

- `.fbs` files are the only source of truth for the wire contract.
- Each module MUST declare one or more FlatBuffers schema files in `schema_files`.
- `atom_api_level` MUST be an integer matching the framework-supported API level for the current
  build.
- `min_atom_version`, when present, MUST be a semver lower bound satisfied by the current framework
  version.
- `ios_min_deployment_target`, when present, MUST use the same `major.minor` format as
  `ios.deployment_target` and the app's configured iOS deployment target MUST be greater than or
  equal to the module's requirement.
- `android_min_sdk`, when present, MUST be `>= 24` and the app's configured Android `min_sdk` MUST
  be greater than or equal to the module's requirement.
- Rule inputs for `schema_files`, `ios_srcs`, and `android_srcs` MAY be package-relative, but the
  emitted metadata MUST normalize them to repo-relative paths.
- Existing FlatBuffers schemas MAY be reused unchanged by listing them in `schema_files`.
- Rust request and response types used at the ABI boundary MUST be generated from `.fbs`.
- Handwritten Rust structs and enums MAY exist as implementation details, but they MUST NOT define
  or evolve the wire contract.
- `MethodSpec.request_table` and `MethodSpec.response_table` MUST be fully qualified FlatBuffers
  table names declared by the module's schema files.
- `depends_on` entries MUST be absolute Bazel labels.

### 6.3 Optional Rust Helper APIs

Rust-authored modules MAY expose typed Rust APIs, helper traits, or proc macros such as `AtomModule`
to reduce boilerplate for library authors.

Rules:

- these helper APIs are library-author conveniences only
- they MUST NOT define canonical module metadata for CNG or module resolution
- they MUST NOT replace Bazel metadata loading as the module discovery mechanism
- they MAY validate local implementation details against the Bazel-owned metadata contract

### 6.4 Module Resolution Rules

- Requested modules are taken from `atom_app(...).modules` in declaration order.
- A module dependency graph is formed using `depends_on`.
- Resolution order MUST be a topological sort of dependencies.
- For ties, declaration order MUST win.
- Duplicate IDs MUST fail with `MODULE_DUPLICATE_ID`.
- Dependency cycles MUST fail with `MODULE_DEPENDENCY_CYCLE`.

### 6.5 Reference Algorithm: Module Resolution

```text
function resolve_modules(requested_modules, app_manifest, framework):
    loaded = []
    seen_ids = set()

    for request in requested_modules in declaration order:
        metadata_target = derive_metadata_target(request.target_label, "_atom_module_metadata")
        metadata_path = bazel_build_and_locate(metadata_target)
        raw = read_text(metadata_path) or error MODULE_MANIFEST_INVALID
        manifest = parse_json(raw) or error MODULE_MANIFEST_INVALID
        validate manifest.target_label == request.target_label or error MODULE_MANIFEST_INVALID
        validate manifest.atom_api_level == framework.atom_api_level or error EXTENSION_INCOMPATIBLE
        validate framework.version satisfies manifest.min_atom_version if present or error EXTENSION_INCOMPATIBLE
        validate app_manifest.ios.deployment_target >= manifest.ios_min_deployment_target if both present or error EXTENSION_INCOMPATIBLE
        validate app_manifest.android.min_sdk >= manifest.android_min_sdk if both present or error EXTENSION_INCOMPATIBLE

        if manifest.id in seen_ids:
            error MODULE_DUPLICATE_ID at manifest.id

        loaded.push((request, manifest))
        seen_ids.add(manifest.id)

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

### 7.6 Runtime Plugin Model

- The runtime MUST provide a runtime plugin registration model that is distinct from native modules.
- Runtime plugins MAY own plugin-local state, observe lifecycle and app events, and emit effects
  through the runtime kernel.
- Runtime plugins MUST use the kernel's dispatch, lifecycle, and task-execution semantics. They MUST
  NOT introduce a second lifecycle model or direct generated-host customization path.
- The runtime kernel MUST remain the single authority for lifecycle transitions, effect completion,
  and dispatch ordering even when higher-level state-management libraries layer on top of it.
- `atom-runtime` MUST expose only destination-agnostic plugin host types. Its public plugin API MUST
  NOT mention iOS, Android, simulator, emulator, device, route-stack, or renderer-specific types.
- Apps MUST opt into runtime plugins in app code by constructing the runtime configuration passed to
  the kernel. `atom-runtime` MUST NOT perform dynamic plugin discovery or hard-code first-party
  plugin registration.
- The app entry crate MUST expose `atom_runtime_config()` or an equivalent generated builder entry
  point that returns the runtime configuration used for startup.
- First-party runtime plugins SHOULD ship as separate crates depending on `atom-runtime`.
- First-party and third-party runtime plugins SHOULD use the same public host API and app-owned
  registration model.
- Navigation SHOULD be implemented as a first-party runtime plugin crate such as `atom-navigation`,
  not as a kernel concern.
- Destination- or platform-specific behavior MAY be packaged as separate adapter/plugin crates
  outside `atom-runtime`, but those crates MUST NOT redefine lifecycle semantics.
- `atom-runtime` MUST NOT own route stacks, screen descriptors, destination discovery, device
  automation, or generated-host customization.

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

### 8.2.1 App-Owned Runtime Registration Handshake

The runtime registration handshake for the initial mobile profile MUST be app-owned.

Rules:

- the app entry crate identified by `entry_crate_name` MUST expose `atom_runtime_config()` or an
  equivalent generated builder entry point
- generated Rust bridge code for iOS and Android MUST call that app-owned registration function
  during `atom_app_init` or JNI init before the runtime is started
- adding or removing a runtime plugin MUST NOT require edits to generated Swift/Kotlin host
  templates beyond the generic framework-owned bridge
- `atom-runtime` MUST remain unaware of concrete plugin crate identities

### 8.3 Generated Method Exports

For each module method declared in normalized module metadata, CNG MUST generate a direct Rust
export with this shape:

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
- `input_flatbuffer`, `out_response_flatbuffer`, and `out_error_flatbuffer` MUST conform to the
  generated FlatBuffers schema for the current app build.
- `config_flatbuffer` MAY be empty in the initial conformance profile. When present, it MUST conform
  to the generated startup schema for the current app build.
- Hosts MUST NOT retain borrowed pointers after the call returns.

### 8.5 Wire Format

Runtime module calls MUST use CNG-generated FlatBuffers, not JSON.

CNG MUST emit:

- `generated/schema/atom.fbs`
- `generated/schema/modules/<module_id>/...` for each declared module schema file

The build layer MUST generate Rust bindings for the runtime side and Swift/Kotlin bindings for
native hosts from the aggregate schema plus all module-owned schema files.

Those generated bindings MUST define the Rust request and response types used by generated
per-method exports and by module implementation code.

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

`config_flatbuffer` is reserved for a CNG-generated startup payload containing normalized app
configuration and platform startup settings. The initial conformance profile MAY pass an empty
payload while relying on the app-owned runtime registration handshake from Section 8.2.1.

The initial bridge profile is synchronous only. Async module work MAY happen inside the Rust
runtime, but it MUST complete behind the synchronous generated method export boundary until a future
ABI revision adds explicit async transport.

## 9. Continuous Native Generation (CNG) Specification

### 9.1 Inputs

CNG consumes:

- normalized `atom_app(...)` metadata
- resolved module metadata
- serialized config/CNG plugin entries from app metadata
- module-owned FlatBuffers schema files
- selected platform set
- build profile

Generated host customization MUST happen through module metadata or config/CNG plugins. Runtime
plugins MUST NOT directly alter generated native host trees.

### 9.1.1 Config/CNG Plugins

Config/CNG plugins MUST be declared in Bazel through plugin-specific Starlark macros that feed
`atom_app(...).config_plugins`, not through runtime discovery.

Each serialized `config_plugins` entry MUST include:

- `id`
- `atom_api_level`
- `min_atom_version`
- `ios_min_deployment_target`
- `android_min_sdk`
- `config`

Rules:

- apps MUST opt into config/CNG plugins through `atom_app(...).config_plugins`
- config/CNG plugin resolution order MUST follow `atom_app(...).config_plugins` declaration order
- config/CNG plugins are build-time extensions only; they MUST NOT be linked into or discovered by
  `atom-runtime`
- config/CNG plugin crates MUST implement the `ConfigPlugin` trait defined by `atom-cng`
- config/CNG plugins MAY contribute files, plist fragments, Android manifest fragments, and Bazel
  resources
- plugin-specific configuration shape MUST be owned by the plugin's Starlark macro and Rust crate,
  not by `atom_app(...)` or `atom-cng`
- config/CNG plugins MUST use deterministic merge semantics compatible with Section 9.2

### 9.1.2 Compatibility Validation

The initial extension compatibility contract MUST be intentionally small and explicit.

Rules:

- the framework MUST declare one supported `atom_api_level` per build
- every module and config/CNG plugin MUST declare `atom_api_level`
- `atom prebuild` MUST fail fast with `EXTENSION_INCOMPATIBLE` if any module or config/CNG plugin
  declares a different `atom_api_level`
- `min_atom_version`, when present, MUST be satisfied by the current framework version
- `ios_min_deployment_target` and `android_min_sdk`, when present, MUST be satisfied by the app
  manifest before host generation begins
- compatibility failures MUST identify the offending target label and field

### 9.2 Merge Rules

| Artifact              | Rule                               | Conflict Behavior                                  |
| --------------------- | ---------------------------------- | -------------------------------------------------- |
| permissions           | set union, lexicographic sort      | never conflicts                                    |
| `plist` maps          | deep merge                         | conflicting scalar values fail with `CNG_CONFLICT` |
| Android manifest maps | deep merge                         | conflicting scalar values fail with `CNG_CONFLICT` |
| entitlements          | deep merge                         | conflicting scalar values fail with `CNG_CONFLICT` |
| generated sources     | concatenate in stable module order | never conflicts                                    |
| init hooks            | stable module order                | never conflicts                                    |

Config/CNG plugin contributions follow the same merge rules. Plugin contributions are merged after
module metadata in plugin registration order. Conflicts between plugin contributions and module
metadata MUST fail with `CNG_CONFLICT`. Plugin-contributed files are copied into the host tree
during emission; plugin-contributed Bazel resources are appended to the platform build rule.

The app manifest MAY later add explicit override sections. Until then, conflicting scalar values
MUST fail generation.

### 9.3 Reference Algorithm: Plan Merge

```text
function build_generation_plan(manifest, modules, framework):
    plan = new GenerationPlan()
    plan.app = manifest.app

    validate_extension_compatibility(modules, manifest.config_plugins, manifest, framework)
        or error EXTENSION_INCOMPATIBLE

    for module in modules in resolved order:
        plan.permissions = union(plan.permissions, module.permissions)
        plan.plist = deep_merge(plan.plist, module.plist) or error CNG_CONFLICT
        plan.android_manifest = deep_merge(plan.android_manifest, module.android_manifest) or error CNG_CONFLICT
        plan.entitlements = deep_merge(plan.entitlements, module.entitlements) or error CNG_CONFLICT
        plan.generated_sources.extend(module.generated_sources)
        plan.module_bindings.append(module.id)

    for entry in manifest.config_plugins in declaration order:
        plugin = instantiate_plugin(entry.id, entry.config)
        plugin.validate() or error
        if manifest.ios.enabled:
            ios_contrib = plugin.contribute_ios(ctx)
            plan.plist = deep_merge(plan.plist, ios_contrib.plist_entries) or error CNG_CONFLICT
            plan.files.extend(ios_contrib.files)
            plan.ios_resources.extend(ios_contrib.bazel_resources)
        if manifest.android.enabled:
            android_contrib = plugin.contribute_android(ctx)
            plan.android_manifest = deep_merge(plan.android_manifest, android_contrib.android_manifest_entries) or error CNG_CONFLICT
            plan.files.extend(android_contrib.files)
            plan.android_resources.extend(android_contrib.bazel_resources)

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

### 9.8 Platform Build Rules

The generated `BUILD.bazel` files MUST produce targets that can be built, installed, and launched on
iOS simulators and Android emulators via Bazel. No Xcode project or Gradle project is required.

#### 9.8.1 iOS Build Target

The generated iOS `BUILD.bazel` MUST use `rules_apple` and `rules_swift` to produce a deployable
`.app` bundle:

```starlark
load("@build_bazel_rules_apple//apple:ios.bzl", "ios_application")
load("@build_bazel_rules_swift//swift:swift.bzl", "swift_library")

swift_library(
    name = "generated_swift",
    srcs = ["AtomAppDelegate.swift", "AtomBindings.swift"],
    module_name = "atom_{slug}_support",
    deps = ["//crates/atom-runtime:atom-runtime-swift-bridge"],
)

ios_application(
    name = "app",
    bundle_id = "{bundle_id}",
    families = ["iphone", "ipad"],
    infoplists = ["Info.generated.plist"],
    minimum_os_version = "{deployment_target}",
    deps = [":generated_swift"],
)
```

Rules:

- The `:app` target MUST be an `ios_application`, not a `swift_binary`.
- The Rust runtime MUST be linked as a static library (`rust_static_library`) through the Swift
  bridge dependency.
- `minimum_os_version` MUST match `ios.deployment_target` from the app manifest.
- `bundle_id` MUST match `ios.bundle_id` from the app manifest.
- Code signing MUST use ad-hoc signing for simulator builds. No Apple Developer account is required
  for simulator-only workflows.

#### 9.8.2 Android Build Target

The generated Android `BUILD.bazel` MUST use `rules_kotlin` and native Android rules to produce a
deployable APK:

```starlark
load("@rules_kotlin//kotlin:jvm.bzl", "kt_jvm_library")

rust_shared_library(
    name = "atom_runtime_jni",
    srcs = ["//templates/atom_runtime_jni:lib.rs"],
    crate_name = "atom_runtime_jni",
    deps = ["//crates/atom-ffi", "//crates/atom-runtime", "{entry_crate}"],
)

kt_jvm_library(
    name = "generated_kotlin",
    srcs = ["AtomApplication.kt", "AtomBindings.kt", "MainActivity.kt"],
)

android_binary(
    name = "app",
    manifest = "AndroidManifest.generated.xml",
    custom_package = "{application_id}",
    deps = [":generated_kotlin"],
    data = [":atom_runtime_jni"],
)
```

Rules:

- The `:app` target MUST be an `android_binary`, not a `java_binary`.
- The Rust runtime MUST be linked as a shared library (`rust_shared_library`) loaded via
  `System.loadLibrary()` in JNI.
- `custom_package` MUST match `android.application_id` from the app manifest.
- `minSdkVersion` in the generated manifest MUST match `android.min_sdk`.
- `targetSdkVersion` in the generated manifest MUST match `android.target_sdk`.

#### 9.8.3 Run and Deploy

`atom run ios` and `atom run android` MUST handle the full build-install-launch cycle, not just
invoke `bazel run`.

iOS deployment sequence:

1. `bazel build //generated/ios/<slug>:app` to produce the `.app` bundle.
2. Boot the default iOS simulator if none is running, or use the currently booted simulator.
3. `xcrun simctl install <device> <path-to-.app>` to install.
4. `xcrun simctl launch <device> <bundle_id>` to launch.

Android deployment sequence:

1. `bazel build //generated/android/<slug>:app` to produce the APK.
2. `adb install -r <path-to-.apk>` to install on the running emulator or connected device.
3. `adb shell am start -n <application_id>/.MainActivity` to launch.

Rules:

- Both commands MUST fail with `EXTERNAL_TOOL_FAILED` if the required platform tools (`xcrun`,
  `adb`) are not available.
- Both commands MUST stream build output to stderr.
- `atom run ios --device <udid>` and `atom run android --device <serial>` MUST support targeting a
  specific simulator, emulator, or connected device.
- When attached to an interactive TTY and `--device` is omitted, `atom run ios` and
  `atom run android` SHOULD offer an interactive destination picker.

#### 9.8.4 Developer Evaluation, Evidence, and Automation

Runnable debug targets MUST expose a framework-owned evaluation surface suitable for both developer
debugging and agent verification.

Definitions:

- A `platform` is a supported app host family such as `ios`, `android`, and future values including
  `macos`, `windows`, `linux`, and `tui`.
- A `destination` is a debuggable runtime instance on a platform, such as an iOS simulator, iOS
  device, Android emulator, Android device, local desktop process, or terminal session.
- A destination advertises a capability set chosen from `launch`, `logs`, `screenshot`, `video`,
  `inspect_ui`, `interact`, and `evaluate`.
- An `evaluation run` is a sequenced execution of launch, wait, inspect, interact, and
  artifact-capture steps against one selected destination, producing a machine-readable proof
  bundle.

Required capabilities:

- destination discovery across supported platforms
- log capture to an explicit output path
- screenshot capture to an explicit output path
- screen recording to an explicit output path
- machine-readable UI inspection with element metadata and screen bounds
- basic interaction: tap, long press, swipe/drag, and text entry
- sequenced evaluation runs that can coordinate the above capabilities and retain proof artifacts

Rules:

- All runnable iOS and Android targets accepted by `atom run` MUST also be surfaced as destinations.
- Additional platforms and destination kinds MAY be added later without weakening the required iOS
  and Android contracts.
- Every destination MUST report a stable identifier, platform, destination kind, display name,
  availability, debug state, and capability set.
- Evidence and interaction commands MUST work against the same runnable targets accepted by
  `atom run`, surfaced through the destination model.
- Log capture MUST be able to collect Atom runtime logs plus relevant host-process logs for the
  selected destination when the underlying platform tooling makes them available.
- Log capture MUST write to a caller-selected output path and MUST fail with
  `AUTOMATION_LOG_CAPTURE_FAILED` when collection was explicitly requested but could not be
  completed.
- Screenshot and video capture MUST be available without requiring Xcode project generation or
  Android Studio project generation.
- UI inspection output MUST be machine-readable and include, at minimum, screen size plus per-node
  bounds, label or text, role or class, visibility, and enabled state when the platform backend can
  supply them.
- Evaluation runs MUST be able to emit a step transcript and an artifact manifest that references
  logs, screenshots, videos, and UI snapshots captured during the run.
- Artifact-producing commands MUST allow caller-selected repo-relative or absolute output paths.
- Video capture SHOULD be startable before the first interaction step and stoppable after the last
  required step so one artifact can prove the full interaction flow.
- The primary automation backend MUST be semantic, not pixel-only.
- iOS automation MUST use a framework-owned XCUITest-based backend or a framework-owned
  WebDriverAgent-compatible backend. Coordinate-only `simctl` helpers are insufficient as the
  primary conformance path.
- Android automation MUST use a framework-owned UI Automator-based or equivalent instrumentation
  backend. `adb shell input` MAY exist only as a fallback for interactions that cannot be expressed
  through the primary backend.
- Coordinate-targeted actions MAY be supported, but semantic element targeting SHOULD be the default
  path exposed to agents.

## 10. CLI Specification

### 10.1 Commands

Required commands:

- `atom prebuild`
- `atom prebuild --dry-run`
- `atom run ios`
- `atom run android`
- `atom destinations`
- `atom devices ios`
- `atom devices android`
- `atom evidence logs`
- `atom evidence screenshot`
- `atom evidence video`
- `atom inspect ui`
- `atom interact`
- `atom evaluate run`
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

- MUST follow the iOS deployment sequence defined in Section 9.8.3

`atom run android`:

- MUST follow the Android deployment sequence defined in Section 9.8.3

`atom destinations`:

- MUST support a machine-readable output mode suitable for agents
- MUST report stable destination identifiers, platform, destination kind, display name,
  availability, debug state, and capability set

`atom devices ios` and `atom devices android`:

- MUST be supported as compatibility commands for mobile-specific destination discovery
- MUST support a machine-readable output mode suitable for agents
- MUST report stable destination identifiers, destination kind, display name, and availability
- MUST only return destinations for the requested mobile platform

`atom evidence screenshot`:

- MUST capture one screenshot from the selected destination
- MUST write the image to the requested output path

`atom evidence video`:

- MUST record a screen video from the selected destination
- MUST write the video to the requested output path

`atom evidence logs`:

- MUST collect logs from the selected destination or launched app process
- MUST write the logs to the requested output path
- SHOULD preserve timestamps and stream ordering when the backend can provide them

`atom inspect ui`:

- MUST emit a machine-readable UI snapshot for the selected destination
- MUST include a screenshot reference or explicit screenshot output path in the snapshot payload

`atom interact`:

- MUST support at least tap, long-press, swipe/drag, and text entry
- SHOULD support semantic element targeting in addition to coordinate targeting
- MUST fail with `AUTOMATION_TARGET_NOT_FOUND` when the requested semantic target cannot be resolved
- MUST fail with `AUTOMATION_UNAVAILABLE` when the selected destination does not support the
  required backend

`atom evaluate run`:

- MUST execute a machine-readable evaluation plan against one selected destination
- MUST allow the plan to request launch, waits, screenshots, video, log capture, UI inspection, and
  interactions
- MUST write a machine-readable artifact manifest plus referenced artifacts under the requested
  output directory
- MUST stop on the first failed required step and surface the underlying automation or tool failure

`atom test`:

- MUST invoke `bazel test //...`

### 10.4 Evaluation Contracts

The exact automation transport is implementation-defined, but the public CLI contract is not.

Evaluation contract rules:

- Destinations are the canonical debug-target abstraction for evaluation.
- Evidence and interaction commands MUST accept the same destination identifiers reported by
  `atom destinations` and `atom devices`.
- Implementations MAY expose additional subcommands, but they MUST preserve the required commands
  from Section 10.1.
- Commands intended for agent use SHOULD offer stable machine-readable output without requiring ANSI
  terminal parsing.
- Screenshot, video, UI inspection, and log artifacts MUST be writable to caller-selected
  repo-relative or absolute output paths.
- Evaluation plans MUST be portable across supported platforms and destination kinds through
  capability discovery rather than through hard-coded simulator-only assumptions.
- An evaluation plan MUST support, at minimum, these step kinds: `launch`, `wait_for_ui`, `tap`,
  `long_press`, `swipe`, `drag`, `type_text`, `screenshot`, `inspect_ui`, `start_video`,
  `stop_video`, and `collect_logs`.
- Interaction and wait steps MUST accept either a semantic target descriptor or an explicit
  coordinate descriptor.
- Evaluation output MUST include a machine-readable bundle manifest with the selected destination
  id, platform, timestamps, executed steps, per-step status, and artifact paths.

### 10.5 Reference Algorithm: `evaluate run`

```text
function cli_evaluate_run(args):
    destination = resolve_destination(args.destination)
    plan = load_evaluation_plan(args.plan)
    require_capabilities(destination, plan)
    session = create_evaluation_session(destination, args.output_dir)

    if plan.collect_logs_on_start:
        session.logs = start_log_capture(destination, args.output_dir)
    if plan.record_video_on_start:
        session.video = start_video_capture(destination, args.output_dir)

    for step in plan.steps:
        execute_step(destination, session, step) or error from underlying command

    finalize_optional_captures(session)
    write_bundle_manifest(session)
    exit 0
```

### 10.6 Reference Algorithm: `prebuild --dry-run`

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

### 11.4 Phase 3: Runnable Mobile Hosts

Required behavior:

- generated iOS `BUILD.bazel` uses `ios_application` from `rules_apple` per Section 9.8.1
- generated Android `BUILD.bazel` uses `android_binary` per Section 9.8.2
- `atom run ios` builds, installs, and launches on an iOS simulator per Section 9.8.3
- `atom run android` builds, installs, and launches on an Android emulator per Section 9.8.3
- no Xcode project or Gradle project is required

Conformance example:

- Input: canonical `hello-atom` app
- Expected output: `atom run ios` launches the app on the booted iOS simulator with Rust lifecycle
  callbacks executing. `atom run android` launches the app on an Android emulator with Rust
  lifecycle callbacks executing via JNI.

### 11.5 Phase 4A: Runtime Kernel

Required behavior:

- module init in resolved order
- runtime lifecycle follows Section 7.2
- invalid transitions return `RUNTIME_TRANSITION_INVALID`

Conformance example:

- Input sequence: `init -> background -> resume -> terminate`
- Expected state sequence:
  `Created -> Initializing -> Running -> Backgrounded -> Running -> Terminating -> Terminated`

### 11.6 Phase 4B: Runtime Plugin SDK and Registration

Required behavior:

- runtime plugins follow Section 7.6
- apps register runtime plugins in app-owned code rather than through kernel-side discovery
- generated hosts do not require per-plugin bootstrap changes
- the same registration path works for framework-owned and third-party-style plugin crates

Conformance example:

- Input: canonical app with one external plugin crate
- Expected output: runtime boots successfully with the plugin registered through app-owned runtime
  config and without changes to `atom-runtime` or generated host templates

### 11.7 Phase 4C: First-Party Plugin Libraries

Required behavior:

- first-party plugins ship as separate crates outside `atom-runtime`
- no first-party plugin types are re-exported from `atom-runtime`
- at least one routing/navigation-style plugin is proven as a library concern rather than a kernel
  concern
- at least one additional non-routing plugin crate proves the model is generic

Conformance example:

- Input: canonical app with `atom-navigation` or `atom-router` plus one additional plugin crate
- Expected output: runtime boots successfully, plugin behavior is available through the shared
  public plugin API, and either plugin can be removed without kernel changes

### 11.8 Phase 5: Config/CNG Plugin System

Required behavior:

- config/CNG plugins are separate crates that implement a `ConfigPlugin` trait owned by `atom-cng`
- `atom-cng` has no knowledge of any specific plugin's domain (icons, splash screens, etc.)
- config/CNG plugins contribute deterministic host customization per Section 9
- config/CNG plugins remain separate from runtime plugins and native modules
- runtime plugins MUST NOT mutate generated native trees directly
- incompatible module or config/CNG plugin metadata fails fast with `EXTENSION_INCOMPATIBLE`
- runtime plugins MUST NOT mutate generated native trees directly
- incompatible module or config/CNG plugin metadata fails fast with `EXTENSION_INCOMPATIBLE`
- the same app may combine runtime plugins, native modules, and config/CNG plugins coherently

#### 11.8.1 Config Plugin Trait

A config plugin crate MUST implement:

- `id() -> &str` returning a unique plugin identifier
- `validate() -> AtomResult<()>` for plugin-owned config validation
- `contribute_ios(ctx) -> AtomResult<PlatformContribution>` for iOS host customization
- `contribute_android(ctx) -> AtomResult<PlatformContribution>` for Android host customization

A `PlatformContribution` MUST contain:

- `files`: list of files to copy or generate into the host tree
- `plist_entries`: plist fragments merged per Section 9.2
- `android_manifest_entries`: manifest fragments merged per Section 9.2
- `bazel_resources`: additional resources for the platform build rule

CNG MUST merge all config plugin contributions after module metadata and before host tree emission.
Conflicts between plugin contributions and module metadata MUST fail with `CNG_CONFLICT`.

#### 11.8.2 Plugin Configuration in Bazel

`atom_app` MUST NOT hard-code plugin-specific fields. Each plugin crate ships a Starlark macro that
returns a config dict. `atom_app` accepts these via a `config_plugins` parameter:

```starlark
load("@atom//crates/atom-cng-app-icon:defs.bzl", "atom_app_icon")

atom_app(
    ...
    config_plugins = [
        atom_app_icon(
            ios = "assets/AppIcon.icon",
            android = "assets/ic_launcher.png",
        ),
    ],
)
```

Each plugin macro MUST return at least
`{"id": "<plugin_id>", "atom_api_level": <n>, "config": {...}}`. It MAY also include
`min_atom_version`, `ios_min_deployment_target`, and `android_min_sdk`. `atom_app` MUST serialize
the list into a `config_plugins` array in the metadata JSON. CNG MUST validate compatibility fields,
instantiate plugins by `id`, pass the opaque `config` to the plugin for parsing and validation, then
call contribution methods.

#### 11.8.3 App Icon Config Plugin (`atom-cng-app-icon`)

The app icon plugin is the first concrete config/CNG plugin. It is a separate crate that implements
`ConfigPlugin` and ships its own `atom_app_icon(...)` Starlark macro.

The plugin owns its config shape. `atom-cng` knows nothing about icon formats.

Per-destination behavior:

- **iOS**: validate the path references a `.icon` bundle containing `icon.json`, copy the bundle
  into `generated/ios/{slug}/AppIcon.icon/`, contribute `CFBundleIconFile = "AppIcon"` to plist, add
  the bundle to `ios_application` resources
- **Android**: validate the source path exists, copy into
  `generated/android/{slug}/src/main/res/mipmap-xxxhdpi/ic_launcher.png`, contribute
  `android:icon="@mipmap/ic_launcher"` to the manifest `<application>` element, add the res
  directory to `android_binary` resource files
- **macOS, Web**: future destinations; omitted until those platforms are supported

When no icon paths are configured, the plugin MUST contribute nothing (no-op).

Conformance example:

- Input: canonical app with one runtime plugin, one native module, and the `atom-cng-app-icon`
  config plugin configured with `ios = "assets/AppIcon.icon"` and
  `android = "assets/ic_launcher.png"`
- Expected output: generated hosts include the correct icon files, plist/manifest reference them,
  build rules include them as resources — all contributed by the plugin crate, not by `atom-cng`
  itself. The runtime plugin remains a runtime-only concern, and no manual edits to generated roots
  are needed. A third party could write a new config plugin crate following the same pattern.

### 11.9 Phase 6: Developer Workflow, Ecosystem, and Evaluation

Required behavior:

- CLI commands behave as defined in Section 10
- generated outputs remain framework-owned
- customization path exists through config/CNG plugins or module metadata without manual edits to
  generated roots
- destination discovery, log capture, screenshot capture, video capture, UI inspection, and basic UI
  interaction work on runnable iOS and Android destinations
- `atom evaluate run` can orchestrate launch, waits, inspection, interactions, and artifact capture
  into one proof bundle
- automation backends are framework-owned and semantic-first per Section 9.8.4
- the evaluation model remains extensible to additional platforms and destination kinds through
  capability discovery
- apps can consume first-party and third-party-style plugin crates through documented workflows

Conformance example:

- Input: run the canonical example app, start an evaluation run with log capture and video
  recording, inspect the UI, tap the primary visible control, and capture a screenshot
- Expected output: framework CLI commands drive the app on a runnable destination, emit
  machine-readable inspection data, write logs plus screenshot or video artifacts, produce an
  artifact manifest, and observe the expected UI transition without manual interaction

### 11.10 Phase 7: Optional Renderer

Renderer behavior is intentionally outside the minimum Atom conformance profile and SHOULD be
specified in a separate renderer spec if and when that work begins.

## 12. Open Questions

- Should app-level override sections be added to resolve plist and manifest merge conflicts?
- Should renderer work live in this spec or a dedicated additive spec?
- Should the iOS automation backend be a framework-owned XCTest bundle directly, or a
  WebDriverAgent-compatible wrapper around the same XCTest primitives?

## 13. Resolved Questions

- **Should Xcode projects be emitted directly, or derived from Bazel later?** Neither for the
  minimum conformance profile. The generated `ios_application` target is built and deployed via
  Bazel and `xcrun simctl`. Xcode project generation via `rules_xcodeproj` MAY be added as a
  convenience in a later phase.
- **Should the runtime artifact be `staticlib`, `cdylib`, or both?** Both. iOS uses `staticlib`
  linked into the Swift binary. Android uses `cdylib` (shared library) loaded via JNI
  `System.loadLibrary()`. See Section 9.8.
- **Should Android-on-Linux be first-class in the first host-capable milestone or follow macOS-first
  bring-up?** CI MUST test on both Linux and macOS. Android builds (APK generation) MUST work on
  Linux. iOS builds require macOS and MUST only run in macOS CI. See Section 14.
- **Should screenshots, recordings, logs, and UI interaction live in external ad hoc scripts or in
  the framework?** In the framework. Agents and humans need a stable, supported CLI surface for
  proof of behavior on real mobile hosts and future platform destinations. See Sections 9.8.4, 10.1,
  and 11.9.

## 14. CI Specification

### 14.1 Job Structure

CI MUST run three job categories:

- **lint**: clippy, format check, shellcheck, actionlint. Runs on Linux.
- **test (linux)**: `bazel test //...` and prebuild dry-run. Runs on Linux.
- **test (macos)**: `bazel test //...` and prebuild dry-run. Runs on macOS.

All three MUST pass before merge to `main`.

### 14.2 Path-Based Filtering

CI jobs MUST only run when changes affect files relevant to that job. This avoids wasting compute on
documentation-only or CI-config-only changes.

Jobs MUST run when any file outside the following documentation-only set is changed:

- `docs/**`
- `*.md` (root-level markdown)
- `LICENSE`

The **lint** job MUST additionally run when any of these are changed:

- `.github/workflows/**`
- `scripts/**`
- `.githooks/**`

The **test** jobs MUST additionally run when any of these are changed:

- `crates/**`
- `bzl/**`
- `templates/**`
- `examples/**`
- `MODULE.bazel`
- `.bazelversion`
- `mise.toml`
- `BUILD.bazel`

When a PR contains only documentation changes, CI SHOULD be skipped via `paths-ignore` on the
workflow trigger.

### 14.3 Platform-Specific Tests

When iOS build targets are introduced (Phase 3), iOS-specific Bazel tests MUST only run in the macOS
CI job. Android build targets MUST be testable on both Linux and macOS CI.

### 14.4 Remote Caching

All CI jobs SHOULD share a remote build cache (BuildBuddy) to avoid redundant compilation across
jobs and platforms.
