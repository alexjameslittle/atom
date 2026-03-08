#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)
cd "$repo_root"

mode=${1:-verify}

lint() {
  bazelisk build --config=clippy //...
  find . -type f \( -name BUILD.bazel -o -name MODULE.bazel -o -name '*.bzl' \) -print0 | \
    xargs -0 buildifier -mode=check
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
    bazelisk build --config=rustfmt-check //...
    lint
    test_suite
    ;;
  *)
    echo "unknown verify mode: $mode" >&2
    exit 64
    ;;
esac
