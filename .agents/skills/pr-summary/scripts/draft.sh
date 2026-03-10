#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

detect_base_ref() {
  for candidate in "${PR_SUMMARY_BASE_REF:-}" origin/main main; do
    [ -n "$candidate" ] || continue
    if git rev-parse --verify "$candidate" >/dev/null 2>&1; then
      echo "$candidate"
      return 0
    fi
  done
  return 1
}

branch=$(git rev-parse --abbrev-ref HEAD)
base_ref=""
merge_base=""
if base_ref=$(detect_base_ref); then
  merge_base=$(git merge-base HEAD "$base_ref")
fi

echo "==> Branch"
echo "  $branch"
if [ -n "$base_ref" ]; then
  echo "  base: $base_ref"
  echo "  merge-base: $merge_base"
else
  echo "  base: (not found; using local worktree state only)"
fi

echo ""
echo "==> Workspace status"
git status --short

echo ""
echo "==> Untracked files"
git ls-files --others --exclude-standard

if [ -n "$merge_base" ]; then
  echo ""
  echo "==> Commits since $base_ref"
  git log --oneline "$merge_base"..HEAD

  echo ""
  echo "==> Changed files since $base_ref"
  git diff --name-status "$merge_base"..HEAD

  echo ""
  echo "==> Diff stat since $base_ref"
  git diff --stat "$merge_base"..HEAD
fi

echo ""
echo "==> Staged diff stat"
git diff --cached --stat

echo ""
echo "==> Unstaged diff stat"
git diff --stat

echo ""
echo "==> PR template"
cat .github/PULL_REQUEST_TEMPLATE.md
