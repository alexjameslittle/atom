# Hello World

This is the canonical Phase 2 consumer fixture for Bazel-first Atom bootstrapping. It includes one
Rust-backed module and one native-only module so the metadata pipeline and generated hosts exercise
both `atom_module(...)` and `atom_native_module(...)`.

Run it from the repository root:

```sh
bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run >/tmp/hello-atom.plan
bazelisk run //:atom -- run ios --target //examples/hello-world/apps/hello_atom:hello_atom
bazelisk run //:atom -- run android --target //examples/hello-world/apps/hello_atom:hello_atom
```
