# Harness Engineering

The repository is wired so local verification and PR verification use the same entrypoints.

## Bootstrap

Run:

```sh
./scripts/bootstrap.sh
```

This expects `mise` to already be installed, then installs the pinned toolchain from [mise.toml](/Users/alexlittle/conductor/workspaces/atom/tehran/mise.toml) and configures Git to use the tracked hooks in [.githooks](/Users/alexlittle/conductor/workspaces/atom/tehran/.githooks).

## Local Guardrails

- `pre-commit` runs formatting and repository-level linters.
- `pre-push` runs the full verification harness.
- `mise run verify` is the canonical local validation command.

The verification harness runs:

- `bazelisk build --config=clippy //...`
- `bazelisk run //:format.check` (rustfmt, ktfmt, swiftformat, buildifier via `aspect_rules_lint`)
- `bazelisk test //...`
- `bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run`
- `shellcheck`
- `actionlint`

## GitHub Guardrails

- [ci.yml](/Users/alexlittle/conductor/workspaces/atom/tehran/.github/workflows/ci.yml) runs the same `mise run ci` harness on pushes and pull requests.
- [dependabot.yml](/Users/alexlittle/conductor/workspaces/atom/tehran/.github/dependabot.yml) keeps workflow dependencies moving.
- [settings.yml](/Users/alexlittle/conductor/workspaces/atom/tehran/.github/settings.yml) captures the intended branch protection policy for repositories that apply GitHub settings from code.
