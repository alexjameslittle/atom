#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)
cd "$repo_root"

mode=${1:-verify}

lint() {
  bazelisk build --config=lint --@aspect_rules_lint//lint:fail_on_violation --keep_going //...
  bazelisk run //:format.check
  shellcheck .githooks/pre-commit .githooks/pre-push .mise/tasks/* scripts/*.sh
  actionlint
}

test_suite() {
  bazelisk test //...
  bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run >/dev/null
}

case "$mode" in
  lint)
    lint
    ;;
  test)
    test_suite
    ;;
  verify)
    lint
    test_suite
    ;;
  *)
    echo "unknown verify mode: $mode" >&2
    exit 64
    ;;
esac
