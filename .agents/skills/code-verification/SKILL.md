---
name: code-verification
description:
  Run Atom's verification harness and surface the first failing lint, test, or prebuild step after
  repository changes.
---

# code-verification

Run the full verification stack when changes affect runtime code, tests, build rules, or generated
output behavior.

## When to use

This skill is **mandatory** when a change touches any of:

- `crates/` (Rust source or tests)
- `bzl/` (Bazel macros and rules)
- `examples/` (consumer app code)
- `scripts/` (verification harness)
- `.github/workflows/` (CI configuration)
- `MODULE.bazel` or `.bazelversion` (dependency or toolchain changes)

## Steps

1. Run `mise exec -- scripts/run.sh`.
2. If any step fails, report the failing step and its output.
3. If the change touches destination discovery, evidence capture, interaction, evaluation, or the
   hello-world demo surface or plain-app proof path, follow this skill with
   `[$destination-discovery](../destination-discovery/SKILL.md)`,
   `[$evidence-capture](../evidence-capture/SKILL.md)`,
   `[$ui-evaluation](../ui-evaluation/SKILL.md)`, or
   `[$examples-auto-run](../examples-auto-run/SKILL.md)` in `evaluate` mode for runtime proof.
4. Do not proceed with a PR or commit until all required verification and runtime proof checks pass.

## What it checks

| Step                | Command                                  | Purpose                                                       |
| ------------------- | ---------------------------------------- | ------------------------------------------------------------- |
| Backend neutrality  | `scripts/check-generic-backend-leaks.sh` | Prevents concrete first-party backend leaks in generic crates |
| Unverified packages | `check_for_unverified_packages`          | Ensures new BUILD dirs are in VERIFY_PACKAGES                 |
| Lint                | `bazelisk build --config=lint ...`       | Clippy and lint rules                                         |
| Format              | `bazelisk run //:format.check`           | Rustfmt via Bazel                                             |
| Shell lint          | `shellcheck`                             | Shell script correctness                                      |
| Action lint         | `actionlint`                             | GitHub Actions correctness                                    |
| Tests               | `bazelisk test`                          | Unit and integration tests                                    |
| Smoke prebuild      | `atom prebuild --dry-run`                | CNG generation doesn't crash                                  |

## Model vs. script split

**Script handles:** running each verification step, collecting exit codes and output, reporting
which step failed.

**Model handles:** interpreting failures, suggesting fixes, deciding whether a failure is related to
the current change or a pre-existing issue.
