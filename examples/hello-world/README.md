# Hello World

This is the canonical Phase 6 consumer for Bazel-first Atom mobile hosts. It includes one
Rust-backed helper crate, two native-only modules, three first-party runtime support crates outside
`atom-runtime`, one first-party config/CNG plugin, and one example-owned demo surface so the
metadata pipeline and generated hosts exercise `atom_module(...)`, `atom_native_module(...)`,
`atom_app(...).config_plugins`, and app-owned `atom_runtime_config()` startup without making
framework automation depend on app-specific generated hooks.

`apps/hello_atom` keeps runtime startup minimal and relies on module metadata plus the demo surface
module for generated-host proof:

```starlark
atom_app(
    name = "hello_atom",
    config_plugins = [
        atom_app_icon(
            ios = "assets/AppIcon.icon",
            android = "assets/ic_launcher.png",
        ),
    ],
    modules = [
        "//examples/hello-world/modules/device_info:device_info",
        "//examples/hello-world/modules/native_echo:native_echo",
        "//examples/hello-world/modules/demo_surface:demo_surface",
    ],
    deps = [
        "//crates/atom-runtime",
    ],
)
```

The `atom_app_icon(...)` config plugin contributes the iOS `.icon` bundle and Android launcher PNG
during prebuild. The example app now targets iOS 18.0 so the generated host matches the working
app-icon setup used elsewhere in the repo. The `demo_surface` native module contributes a
deterministic text field, button, and visible state changes through app-owned Swift/Kotlin sources,
while `hello_atom_plain` omits that module to prove launch, inspection, and evidence capture work
without any demo-specific UI.

The app crate provides an empty runtime config for the generated singleton bridge:

```rust
pub fn atom_runtime_config() -> atom_runtime::RuntimeConfig {
    atom_runtime::RuntimeConfig::builder().build()
}
```

The surrounding example crates now exercise the module/runtime boundary in plain Rust:

- `modules/device_info` exposes `get() -> DeviceInfo` as a plain Rust helper
- `crates/atom-navigation` and `crates/atom-analytics` mirror app-owned state into `atom_runtime::*`
  free functions when the runtime is available
- `plugins/lifecycle_logger` records state, dispatches events, and performs async warmup through the
  public runtime singleton API without `PluginContext` or runtime-side module registration

That keeps the proof of state changes and async work inside the same public runtime API surface used
by first-party and third-party support crates.

Run it from the repository root:

```sh
mise run ios
mise run android
mise run ios -- --detach
mise run android -- --detach
mise run ios -- --destination 00008130-001431E90A78001C
mise run android -- --destination avd:atom_35
mise run ios -- --detach --destination 00008130-001431E90A78001C
mise run android -- --detach --destination avd:atom_35
mise exec -- bazelisk run //:atom -- stop --platform ios --target //examples/hello-world/apps/hello_atom:hello_atom
mise exec -- bazelisk run //:atom -- stop --platform android --target //examples/hello-world/apps/hello_atom:hello_atom --destination avd:atom_35

bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run >/tmp/hello-atom.plan
bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom_plain --dry-run >/tmp/hello-atom-plain.plan
bazelisk run //:atom -- destinations --platform ios --json
bazelisk run //:atom -- run --platform ios --target //examples/hello-world/apps/hello_atom:hello_atom
bazelisk run //:atom -- run --platform android --target //examples/hello-world/apps/hello_atom:hello_atom
bazelisk run //:atom -- run --platform ios --target //examples/hello-world/apps/hello_atom:hello_atom --detach
bazelisk run //:atom -- run --platform android --target //examples/hello-world/apps/hello_atom:hello_atom --detach
bazelisk run //:atom -- run --platform ios --target //examples/hello-world/apps/hello_atom:hello_atom --destination 00008130-001431E90A78001C
bazelisk run //:atom -- run --platform android --target //examples/hello-world/apps/hello_atom:hello_atom --destination avd:atom_35
bazelisk run //:atom -- stop --platform ios --target //examples/hello-world/apps/hello_atom:hello_atom --destination 00008130-001431E90A78001C
bazelisk run //:atom -- stop --platform android --target //examples/hello-world/apps/hello_atom:hello_atom --destination avd:atom_35
bazelisk run //:atom -- inspect ui --platform ios --target //examples/hello-world/apps/hello_atom:hello_atom --destination SIM-123 --output /tmp/hello-atom-ui.json
bazelisk run //:atom -- evaluate run --platform ios --target //examples/hello-world/apps/hello_atom:hello_atom --destination SIM-123 --plan examples/hello-world/evaluation/demo_surface_plan.json --artifacts-dir /tmp/hello-atom-eval
```

When `--destination` is omitted and the command is running in an interactive terminal, Atom now
prompts you to choose a simulator, emulator, or connected device.

The standalone `atom inspect ui --platform <platform>`, `atom interact --platform <platform>`, and
`atom evidence ... --platform <platform>` commands reuse the current foreground app state when the
selected target is already running, so ad hoc debugging does not force a relaunch before collecting
artifacts.

`atom run --platform <platform>` streams logs by default for manual debugging. Use `--detach` when
you want the app to keep running without a live terminal session. Detached launch now returns only
after the app is inspectable for follow-on `inspect`, `interact`, or `evidence` commands. Use
`atom stop --platform <platform>` to stop a disposable session without uninstalling the app or
shutting down the simulator/emulator.

For standalone video capture, prefer `.mov` output paths on iOS and `.mp4` output paths on Android.
Proof bundles normalize their own artifact names automatically.
