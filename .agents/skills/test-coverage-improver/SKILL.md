---
name: test-coverage-improver
description:
  Audit changed files against nearby test targets so Atom changes land with the smallest high-value
  test additions instead of silent coverage regressions.
---

# test-coverage-improver

Find the smallest useful test additions for the current change instead of waiting for a regression
to prove the gap.

## When to use

Use this skill when:

- Behavior changes land in `crates/`, `bzl/`, `examples/`, or generated-output logic.
- The diff adds branches, validation rules, parsing, or new CLI behavior.
- The change feels under-tested even if the existing suite still passes.

## Steps

1. Run `scripts/audit.sh`.
2. For each changed file, identify the nearest existing test target or prove that none exists.
3. Suggest the smallest test additions that would lock in the new behavior.
4. Prefer focused unit or integration tests over broad hand-wavy “more coverage” advice.

## Output

- Changed files grouped by likely test target.
- Files with no obvious nearby tests.
- Specific test additions or updates that should accompany the change.

## Model vs. script split

**Script handles:** collecting changed files, discovering nearby Bazel test targets, and listing
existing test files.

**Model handles:** identifying the real coverage gaps, choosing the highest-value tests, and
drafting concrete test cases.
