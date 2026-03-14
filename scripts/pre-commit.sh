#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)
cd "$repo_root"

mise run fmt-check
mise exec -- shellcheck .githooks/pre-commit .githooks/pre-push .mise/tasks/* scripts/*.sh ./install.sh tools/install/*.sh
mise exec -- actionlint
