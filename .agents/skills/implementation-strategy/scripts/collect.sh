#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

echo "==> Branch"
git rev-parse --abbrev-ref HEAD

echo ""
echo "==> Workspace status"
git status --short

echo ""
echo "==> Recent commits"
git log --oneline -5

echo ""
echo "==> Core docs"
for path in \
  AGENTS.md \
  docs/README.md \
  docs/architecture.md \
  docs/harness.md \
  docs/plan.md \
  SPEC.md \
  .github/PULL_REQUEST_TEMPLATE.md \
  scripts/verify.sh
do
  [ -e "$path" ] && echo "  $path"
done

echo ""
echo "==> Crate inventory"
find crates -mindepth 1 -maxdepth 1 -type d | sort | sed 's#^#  #'

echo ""
echo "==> Example inventory"
find examples -name 'BUILD*' | sort | sed 's#^#  #'

echo ""
echo "==> Skill inventory"
find .agents/skills -mindepth 1 -maxdepth 1 -type d | sort | sed 's#^#  #'

echo ""
echo "==> Verification entrypoints"
printf '%s\n' \
  "  ./scripts/bootstrap.sh" \
  "  mise run fmt" \
  "  mise run verify" \
  "  bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run"
