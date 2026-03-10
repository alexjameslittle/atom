#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

EXAMPLE_TARGET="//examples/hello-world/apps/hello_atom:hello_atom"
DEFAULT_PLAN="examples/hello-world/evaluation/automation_fixture_plan.json"
mode=${1:-smoke}

echo "==> examples-auto-run mode: $mode"

case "$mode" in
  smoke)
    bazelisk run //:atom -- prebuild --target "$EXAMPLE_TARGET" --dry-run
    ;;
  generated-tree)
    bazelisk run //:atom -- prebuild --target "$EXAMPLE_TARGET"
    if [ -d generated ]; then
      find generated -maxdepth 3 -type f | sort | sed 's#^#  #'
    else
      echo "  generated/ not found"
    fi
    ;;
  evaluate)
    destination=${2:?destination id required}
    artifacts_dir=${3:?artifacts dir required}
    plan=${4:-$DEFAULT_PLAN}
    bazelisk run //:atom -- prebuild --target "$EXAMPLE_TARGET"
    bazelisk run //:atom -- evaluate run \
      --target "$EXAMPLE_TARGET" \
      --destination "$destination" \
      --plan "$plan" \
      --artifacts-dir "$artifacts_dir"
    ;;
  android)
    ./scripts/verify.sh build-android
    ;;
  ios)
    ./scripts/verify.sh build-ios
    ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "expected one of: smoke, generated-tree, evaluate, android, ios" >&2
    exit 64
    ;;
esac
