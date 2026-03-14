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

After bootstrap, run `bazelisk run //:atom -- doctor` for a fast environment sanity check before
full verification. `atom doctor` reports pinned Bazel/Rust/mise status plus backend-owned iOS and
Android readiness probes, and only critical toolchain failures make it exit non-zero.

## Local Guardrails

- `pre-commit` runs formatting and repository-level linters.
- `pre-push` runs lint plus host tests and the prebuild dry-run. Example app builds rely on CI.
- `mise run verify` is the canonical local validation command.
- When invoking repo verification scripts directly, prefix them with `mise exec --`.

The verification harness runs:

- `mise exec -- bazelisk build --config=lint --@aspect_rules_lint//lint:fail_on_violation //...`
  (clippy via `aspect_rules_lint`)
- `mise exec -- bazelisk run //:format.check` (rustfmt, ktfmt, swiftformat, buildifier, prettier via
  `aspect_rules_lint`)
- `mise exec -- bazelisk test //...`
- `mise exec -- bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run`
- `sh scripts/verify-scaffold-project.sh` (builds the CLI binary, scaffolds a temp project, points
  it at the checkout under test, and verifies the dry-run plan includes iOS + Android outputs)
- `mise exec -- shellcheck`
- `mise exec -- actionlint`

## GitHub Guardrails

CI runs the main verification matrix in [../.github/workflows/ci.yml](../.github/workflows/ci.yml)
and a standalone CLI artifact workflow in
[../.github/workflows/cli-binary.yml](../.github/workflows/cli-binary.yml), plus a tag-triggered
GitHub prerelease workflow in
[../.github/workflows/github-release.yml](../.github/workflows/github-release.yml), all sharing the
same toolchain setup action.

The verification matrix currently runs:

- **lint** (Linux): clippy, format check, shellcheck, actionlint
- **test (linux)**: host tests, example prebuild dry-run, scaffolded-project CLI dry-run
- **build example apps (android)** (Linux): prebuild plus Android example app build
- **build atom macOS arm64 binary** (macOS): Bazel build plus standalone CLI artifact upload
- **GitHub Release** (macOS on `v*` tags): reuses the standalone CLI build and publishes prerelease
  assets plus release notes

All jobs must pass before merge.

- [../.github/workflows/ci.yml](../.github/workflows/ci.yml) defines the CI matrix.
- [../.github/workflows/github-release.yml](../.github/workflows/github-release.yml) defines
  version-tag prerelease publishing.
- [../.github/dependabot.yml](../.github/dependabot.yml) keeps workflow dependencies moving.
- [../.github/settings.yml](../.github/settings.yml) captures the intended branch protection policy
  for repositories that apply GitHub settings from code.
