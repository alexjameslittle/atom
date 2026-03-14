# Harness Engineering

The repository is wired so local verification and PR verification use the same entrypoints.

## Bootstrap

Run:

```sh
./scripts/bootstrap.sh
```

This expects `mise` to already be installed, then installs the pinned toolchain from
[../mise.toml](../mise.toml) and configures Git to use the tracked hooks in
[../.githooks](../.githooks). Bootstrap also installs the pinned `agent-device` CLI via npm so the
framework-owned automation backend is available on `PATH`. On macOS hosts with Homebrew available,
bootstrap additionally installs the iOS companion tooling (`idb_companion` plus `fb-idb`) needed by
the underlying Apple-side automation stack, and `scripts/setup-android-sdk.sh` installs the Android
SDK tooling (`adb`, emulator, platform tools) that `agent-device` wraps on Android hosts. Atom
scopes `agent-device` daemon state under `cng-output/agent-device-state` so sessions stay
workspace-local instead of leaking through a shared home-directory daemon.

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

CI runs five parallel job executions sharing a BuildBuddy remote cache:

- **lint** (Linux): clippy, format check, shellcheck, actionlint
- **test (linux)**: host tests, prebuild dry-run
- **test (macos)**: host tests, prebuild dry-run
- **build example apps (ios)** (macOS): prebuild plus iOS example app build
- **build example apps (android)** (Linux): prebuild plus Android example app build

All jobs must pass before merge.

- [../.github/workflows/ci.yml](../.github/workflows/ci.yml) defines the CI matrix.
- [../.github/dependabot.yml](../.github/dependabot.yml) keeps workflow dependencies moving.
- [../.github/settings.yml](../.github/settings.yml) captures the intended branch protection policy
  for repositories that apply GitHub settings from code.
