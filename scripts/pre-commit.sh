#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)
cd "$repo_root"

mise run fmt-check
find . -type f \( -name BUILD.bazel -o -name MODULE.bazel -o -name '*.bzl' \) -print0 | \
  xargs -0 buildifier -mode=check
shellcheck .githooks/pre-commit .githooks/pre-push .mise/tasks/* scripts/*.sh
actionlint
