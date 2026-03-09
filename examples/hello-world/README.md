# Hello World

This is the canonical Phase 4 consumer fixture for Bazel-first Atom mobile hosts. It includes one
Rust-backed module, one native-only module, and two first-party runtime plugin crates outside
`atom-runtime` so the metadata pipeline and generated hosts exercise `atom_module(...)`,
`atom_native_module(...)`, and app-owned `atom_runtime_config()` registration.

`apps/hello_atom` consumes the plugins as normal Bazel Rust dependencies:

```starlark
atom_app(
    name = "hello_atom",
    deps = [
        "//crates/atom-analytics",
        "//crates/atom-navigation",
        "//crates/atom-runtime",
    ],
)
```

The app crate opts into the plugins in Rust:

```rust
pub fn atom_runtime_config() -> atom_runtime::RuntimeConfig {
    let navigation = atom_navigation::NavigationPlugin::new("home");
    navigation.handle().push("device_info");

    let analytics = atom_analytics::AnalyticsPlugin::new("hello_atom");
    analytics.handle().track("runtime_configured");

    atom_runtime::RuntimeConfig::builder()
        .plugin(navigation)
        .plugin(analytics)
        .build()
}
```

The example-only `plugins/lifecycle_logger` crate remains available as a third-party-style reference
plugin, but the canonical app wiring now proves that first-party navigation and analytics behavior
stays outside the runtime kernel.

Run it from the repository root:

```sh
mise run ios
mise run android
mise run ios -- --device 00008130-001431E90A78001C
mise run android -- --device emulator-5554

bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run >/tmp/hello-atom.plan
bazelisk run //:atom -- run ios --target //examples/hello-world/apps/hello_atom:hello_atom
bazelisk run //:atom -- run android --target //examples/hello-world/apps/hello_atom:hello_atom
bazelisk run //:atom -- run ios --target //examples/hello-world/apps/hello_atom:hello_atom --device 00008130-001431E90A78001C
bazelisk run //:atom -- run android --target //examples/hello-world/apps/hello_atom:hello_atom --device emulator-5554
```

When `--device` is omitted and the command is running in an interactive terminal, Atom now prompts
you to choose a simulator, emulator, or connected device.
