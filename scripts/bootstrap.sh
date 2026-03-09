#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)

if ! command -v mise >/dev/null 2>&1; then
  echo "mise is required; install it from https://mise.jdx.dev/" >&2
  exit 69
fi

# Trust before cd-ing into the repo so mise's shell hook doesn't error
mise trust -y "$repo_root/mise.toml" 2>/dev/null || true
cd "$repo_root"

mise install
eval "$(mise env)"
./scripts/setup-android-sdk.sh
./scripts/install-hooks.sh

echo "bootstrap complete"
