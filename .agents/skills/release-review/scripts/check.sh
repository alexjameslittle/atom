#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

detect_base_ref() {
  for candidate in "${RELEASE_REVIEW_BASE_REF:-}" origin/main main; do
    [ -n "$candidate" ] || continue
    if git rev-parse --verify "$candidate" >/dev/null 2>&1; then
      echo "$candidate"
      return 0
    fi
  done
  return 1
}

collect_changed_files() {
  if base_ref=$(detect_base_ref); then
    merge_base=$(git merge-base HEAD "$base_ref")
    git diff --name-only "$merge_base"..HEAD
  fi
  git diff --name-only
  git diff --cached --name-only
  git ls-files --others --exclude-standard
}

echo "==> Branch state"
git rev-parse --abbrev-ref HEAD
git status --short

echo ""
echo "==> Release-surface files"
collect_changed_files | awk 'NF' | sort -u | \
  grep -E '^(SPEC\.md|AGENTS\.md|docs/|crates/atom-(ffi|manifest|modules|runtime|cng|cli|deploy)|examples/|bzl/|\.github/)' || \
  echo "  (no release-surface files detected)"

echo ""
echo "==> Compatibility and contract markers"
rg -n 'atom_api_level|config_plugins|AtomErrorCode|RuntimeState|ExitCode|compatibility' \
  SPEC.md \
  AGENTS.md \
  docs/architecture.md \
  docs/plan.md \
  crates/atom-ffi \
  crates/atom-modules \
  crates/atom-runtime \
  crates/atom-cli 2>/dev/null || echo "  (no markers found)"

echo ""
echo "==> Release checklist dependencies"
printf '%s\n' \
  "  code-verification" \
  "  docs-sync" \
  "  spec-sync" \
  "  examples-auto-run"
