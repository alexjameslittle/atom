# Harness Engineering

The repository is wired so local verification and PR verification use the same entrypoints.

## Bootstrap

Run:

```sh
./scripts/bootstrap.sh
```

This expects `mise` to already be installed, then installs the pinned toolchain from
[../mise.toml](../mise.toml) and configures Git to use the tracked hooks in
[../.githooks](../.githooks). On macOS hosts with Homebrew available, bootstrap also installs the
`idb` companion (`idb_companion`) from `facebook/fb` and installs the `fb-idb` CLI into the Homebrew
prefix so the `idb` command is available on `PATH`.

## Local Guardrails

- `pre-commit` runs formatting and repository-level linters.
- `pre-push` runs lint plus host tests and the prebuild dry-run. Example app builds rely on CI.
- `mise run verify` is the canonical local validation command.

The verification harness runs:

- `bazelisk build --config=lint --@aspect_rules_lint//lint:fail_on_violation //...` (clippy via
  `aspect_rules_lint`)
- `bazelisk run //:format.check` (rustfmt, ktfmt, swiftformat, buildifier, prettier via
  `aspect_rules_lint`)
- `bazelisk test //...`
- `bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run`
- `shellcheck`
- `actionlint`

## GitHub Guardrails

CI runs the main verification matrix in [../.github/workflows/ci.yml](../.github/workflows/ci.yml)
and a standalone CLI artifact workflow in
[../.github/workflows/cli-binary.yml](../.github/workflows/cli-binary.yml), both sharing the same
toolchain setup action.

The verification matrix currently runs:

- **lint** (Linux): clippy, format check, shellcheck, actionlint
- **test (linux)**: host tests, prebuild dry-run
- **build example apps (android)** (Linux): prebuild plus Android example app build
- **build atom macOS arm64 binary** (macOS): Bazel build plus standalone CLI artifact upload

All jobs must pass before merge.

- [../.github/workflows/ci.yml](../.github/workflows/ci.yml) defines the CI matrix.
- [../.github/dependabot.yml](../.github/dependabot.yml) keeps workflow dependencies moving.
- [../.github/settings.yml](../.github/settings.yml) captures the intended branch protection policy
  for repositories that apply GitHub settings from code.
