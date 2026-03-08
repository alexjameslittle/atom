# Harness Engineering

The repository is wired so local verification and PR verification use the same entrypoints.

## Bootstrap

Run:

```sh
./scripts/bootstrap.sh
```

This expects `mise` to already be installed, then installs the pinned toolchain from
[mise.toml](/Users/alexlittle/conductor/workspaces/atom/tehran/mise.toml) and configures Git to use
the tracked hooks in [.githooks](/Users/alexlittle/conductor/workspaces/atom/tehran/.githooks).

## Local Guardrails

- `pre-commit` runs formatting and repository-level linters.
- `pre-push` runs the full verification harness.
- `mise run verify` is the canonical local validation command.

The verification harness runs:

- `bazelisk build --config=clippy //...`
- `bazelisk run //:format.check` (rustfmt, ktfmt, swiftformat, buildifier, prettier via
  `aspect_rules_lint`)
- `bazelisk test //...`
- `bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run`
- `shellcheck`
- `actionlint`

## GitHub Guardrails

CI runs three parallel jobs sharing a BuildBuddy remote cache:

- **lint** (Linux): clippy, format check, shellcheck, actionlint
- **test (linux)**: host tests, prebuild dry-run
- **test (macos)**: host tests, prebuild dry-run

All three must pass before merge.

- [ci.yml](/Users/alexlittle/conductor/workspaces/atom/tehran/.github/workflows/ci.yml) defines the
  CI matrix.
- [dependabot.yml](/Users/alexlittle/conductor/workspaces/atom/tehran/.github/dependabot.yml) keeps
  workflow dependencies moving.
- [settings.yml](/Users/alexlittle/conductor/workspaces/atom/tehran/.github/settings.yml) captures
  the intended branch protection policy for repositories that apply GitHub settings from code.
