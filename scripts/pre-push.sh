#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)
cd "$repo_root"

exec ./scripts/verify.sh verify
