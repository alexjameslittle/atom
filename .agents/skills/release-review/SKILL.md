---
name: release-review
description:
  Run a final repo-level release checklist so docs, spec, verification, examples, and compatibility
  surfaces are aligned before a release candidate or release-critical merge.
---

# release-review

Do one last pass across Atom's release surfaces before a release candidate or another high-stakes
handoff.

## When to use

Use this skill when:

- A release candidate is being prepared.
- A change touches compatibility, CLI behavior, metadata fields, error codes, or generated output.
- The branch is large enough that one final review pass is cheaper than fixing missed drift later.

## Steps

1. Run `scripts/check.sh`.
2. Run `code-verification`, `docs-sync`, `spec-sync`, and `examples-auto-run` if their triggers are
   active for the branch.
3. Confirm the release blockers list is empty before calling the branch ready.
4. Write down any residual risks instead of assuming release readiness.

## What to review

- Verification status and any skipped checks.
- SPEC/docs alignment for changed behavior.
- Compatibility markers and user-facing contracts.
- Example app health for framework-facing changes.
- Release blockers that still need human signoff.

## Model vs. script split

**Script handles:** collecting changed release surfaces, compatibility markers, and a checklist of
dependent skills.

**Model handles:** deciding whether the branch is actually release-ready, identifying blockers, and
summarizing residual risk.
