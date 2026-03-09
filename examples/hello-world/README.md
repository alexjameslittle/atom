# Hello World

This is the canonical Phase 3 consumer fixture for Bazel-first Atom mobile hosts. It includes one
Rust-backed module, one native-only module, and one runtime plugin crate outside `atom-runtime` so
the metadata pipeline and generated hosts exercise `atom_module(...)`, `atom_native_module(...)`,
and app-owned `atom_runtime_config()` registration.

`apps/hello_atom` consumes the plugin as a normal Bazel Rust dependency:

```starlark
atom_app(
    name = "hello_atom",
    deps = [
        "//crates/atom-runtime",
        "//examples/hello-world/plugins/lifecycle_logger",
    ],
)
```

The app crate opts into the plugin in Rust:

```rust
pub fn atom_runtime_config() -> atom_runtime::RuntimeConfig {
    atom_runtime::RuntimeConfig::builder()
        .plugin(hello_world_lifecycle_logger::LifecycleLoggerPlugin::new())
        .build()
}
```

Run it from the repository root:

```sh
bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run >/tmp/hello-atom.plan
bazelisk run //:atom -- run ios --target //examples/hello-world/apps/hello_atom:hello_atom
bazelisk run //:atom -- run android --target //examples/hello-world/apps/hello_atom:hello_atom
bazelisk run //:atom -- run ios --target //examples/hello-world/apps/hello_atom:hello_atom --device 00008130-001431E90A78001C
bazelisk run //:atom -- run android --target //examples/hello-world/apps/hello_atom:hello_atom --device emulator-5554
```

When `--device` is omitted and the command is running in an interactive terminal, Atom now prompts
you to choose a simulator, emulator, or connected device.
