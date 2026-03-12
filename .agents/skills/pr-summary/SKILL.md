---
name: pr-summary
description:
  Draft a PR title, summary, verification section, and handoff note from the current branch diff and
  repository conventions.
---

# pr-summary

Turn the current branch state into a clean pull request summary or end-of-task handoff.

## When to use

Use this skill when:

- A change is ready to hand back to the user.
- You need a PR title/body that follows the repo template.
- You want a concise summary of what changed, why, and how it was verified.

## Steps

1. Run `mise exec -- scripts/draft.sh`.
2. Review `.github/PULL_REQUEST_TEMPLATE.md` and fill it with the real branch changes.
3. Keep the final summary outcome-focused: what changed, verification run, and any residual risk.
4. If verification is incomplete, say so explicitly instead of implying the branch is ready.

## Output

- PR title candidates.
- Summary bullets grounded in the actual diff.
- Verification bullets aligned with the repo template.
- A short handoff note when the work is not yet PR-ready.

## Model vs. script split

**Script handles:** collecting branch/base information, changed files, diff stats, commit history,
and the PR template.

**Model handles:** writing the title/body, grouping edits into coherent themes, and deciding what
verification and risks belong in the handoff.
