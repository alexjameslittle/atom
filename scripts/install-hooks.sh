#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)
cd "$repo_root"

git config core.hooksPath .githooks
chmod +x .githooks/pre-commit .githooks/pre-push

echo "git hooks installed from .githooks"
