#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)
cd "$repo_root"

if ! command -v mise >/dev/null 2>&1; then
  echo "mise is required; install it from https://mise.jdx.dev/" >&2
  exit 69
fi

mise trust -y
mise install
./scripts/install-hooks.sh

echo "bootstrap complete"
