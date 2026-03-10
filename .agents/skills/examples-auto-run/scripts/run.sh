#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

EXAMPLE_TARGET="//examples/hello-world/apps/hello_atom:hello_atom"
DEFAULT_PLAN="examples/hello-world/evaluation/automation_fixture_plan.json"
mode=${1:-smoke}

detect_generated_root() {
  for candidate in cng-output generated; do
    if [ -d "$candidate/ios" ] || [ -d "$candidate/android" ] || [ -d "$candidate/schema" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  return 1
}

echo "==> examples-auto-run mode: $mode"

case "$mode" in
  smoke)
    bazelisk run //:atom -- prebuild --target "$EXAMPLE_TARGET" --dry-run
    ;;
  generated-tree)
    bazelisk run //:atom -- prebuild --target "$EXAMPLE_TARGET"
    if generated_root=$(detect_generated_root); then
      echo "  generated root: $generated_root"
      find "$generated_root" -maxdepth 3 -type f | sort | sed 's#^#  #'
    else
      echo "  generated host tree not found"
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
