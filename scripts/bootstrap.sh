#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)

if ! command -v mise >/dev/null 2>&1; then
  echo "mise is required; install it from https://mise.jdx.dev/" >&2
  exit 69
fi

# Trust this worktree and register the parent directory so future
# worktrees are auto-trusted by mise's shell hook.
mise trust -y "$repo_root/mise.toml" 2>/dev/null || true
mise settings add trusted_config_paths "$repo_root" 2>/dev/null || true
cd "$repo_root"

mise install
./scripts/install-hooks.sh

echo "bootstrap complete"
