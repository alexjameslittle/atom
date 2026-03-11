#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)
cd "$repo_root"

pattern='(^|[^[:alnum:]_])(ios|android)([^[:alnum:]_]|$)|contribute_ios|contribute_android|DestinationPlatform|DestinationKind|atom-backend-ios|atom-backend-android'
set -- crates/atom-backends crates/atom-cng crates/atom-deploy

echo "==> Checking generic crates for concrete backend leaks"
if rg -n --glob '*.rs' --glob 'BUILD.bazel' --glob 'SKILL.md' --glob 'README.md' \
  "$pattern" "$@"
then
  echo ""
  echo "ERROR: generic crates must stay backend-neutral; move concrete first-party references into backend crates or schema-owning crates." >&2
  exit 1
fi

echo "Generic crate backend-neutrality check passed."
