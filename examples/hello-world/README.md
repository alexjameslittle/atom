# Hello World

This is the canonical Phase 1 consumer fixture for Bazel-first `atom prebuild --dry-run`. It includes one Rust module and one native-only module so the metadata pipeline exercises both `atom_module(...)` and `atom_native_module(...)`.

Run it from the repository root:

```sh
bazel run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run >/tmp/hello-atom.plan
```
