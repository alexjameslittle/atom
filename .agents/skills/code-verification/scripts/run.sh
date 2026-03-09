#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../.." && pwd)
cd "$repo_root"

failed=""

step() {
  name="$1"
  shift
  echo "==> $name"
  if "$@"; then
    echo "    PASS: $name"
  else
    echo "    FAIL: $name (exit $?)"
    failed="$failed $name"
  fi
}

step "lint" ./scripts/verify.sh lint
step "test" ./scripts/verify.sh test
step "smoke-prebuild" bazelisk run //:atom -- prebuild \
  --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run

if [ -n "$failed" ]; then
  echo ""
  echo "FAILED steps:$failed"
  exit 1
fi

echo ""
echo "All verification steps passed."
