# test-coverage-improver references

Files this skill usually needs to inspect:

- `scripts/verify.sh` — canonical verification modes
- `crates/*/BUILD.bazel` — Bazel test targets
- `crates/*/tests/` — integration test files
- `examples/hello-world/` — example app validation surface
- `bzl/atom/` — macro and metadata logic that may need smoke coverage
