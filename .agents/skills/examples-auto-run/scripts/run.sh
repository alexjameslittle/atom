#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

EXAMPLE_TARGET="//examples/hello-world/apps/hello_atom:hello_atom"
PLAIN_TARGET="//examples/hello-world/apps/hello_atom:hello_atom_plain"
DEFAULT_PLAN="examples/hello-world/evaluation/demo_surface_plan.json"

run_atom() {
  mise exec -- bazelisk run //:atom -- "$@"
}

mode=${1:-smoke}
shift || true

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
    run_atom prebuild --target "$PLAIN_TARGET" --dry-run
    ;;
  generated-tree)
    run_atom prebuild --target "$EXAMPLE_TARGET"
    if generated_root=$(detect_generated_root); then
      echo "  generated root: $generated_root"
      find "$generated_root" -maxdepth 3 -type f | sort | sed 's#^#  #'
    else
      echo "  generated host tree not found"
    fi
    ;;
  evaluate)
    platform=ios
    case "${1:-}" in
      ios|android)
        platform=$1
        shift
        ;;
    esac
    destination=${1:?destination id required}
    artifacts_dir=${2:?artifacts dir required}
    plan=${3:-$DEFAULT_PLAN}
    run_atom prebuild --target "$EXAMPLE_TARGET"
    run_atom evaluate run \
      --platform "$platform" \
      --target "$EXAMPLE_TARGET" \
      --destination "$destination" \
      --plan "$plan" \
      --artifacts-dir "$artifacts_dir"
    ;;
  android)
    mise exec -- ./scripts/verify.sh build-android
    ;;
  ios)
    mise exec -- ./scripts/verify.sh build-ios
    ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "expected one of: smoke, generated-tree, evaluate, android, ios" >&2
    exit 64
    ;;
esac
