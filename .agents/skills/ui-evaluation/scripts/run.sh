#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

EXAMPLE_TARGET="//examples/hello-world/apps/hello_atom:hello_atom"
DEFAULT_PLAN="examples/hello-world/evaluation/demo_surface_plan.json"

run_atom() {
  mise exec -- bazelisk run //:atom -- "$@"
}

mode=${1:-}
shift || true

case "$mode" in
  tap)
    platform=ios
    case "${1:-}" in
      ios|android) platform=$1; shift ;;
    esac
    destination=${1:?destination id required}
    target_id=${2:?target id required}
    target=${3:-$EXAMPLE_TARGET}
    run_atom interact tap --platform "$platform" --target "$target" --destination "$destination" --target-id "$target_id"
    ;;
  long-press)
    platform=ios
    case "${1:-}" in
      ios|android) platform=$1; shift ;;
    esac
    destination=${1:?destination id required}
    target_id=${2:?target id required}
    target=${3:-$EXAMPLE_TARGET}
    run_atom interact long-press --platform "$platform" --target "$target" --destination "$destination" --target-id "$target_id"
    ;;
  swipe)
    platform=ios
    case "${1:-}" in
      ios|android) platform=$1; shift ;;
    esac
    destination=${1:?destination id required}
    x=${2:?x required}
    y=${3:?y required}
    target=${4:-$EXAMPLE_TARGET}
    run_atom interact swipe --platform "$platform" --target "$target" --destination "$destination" --x "$x" --y "$y"
    ;;
  drag)
    platform=ios
    case "${1:-}" in
      ios|android) platform=$1; shift ;;
    esac
    destination=${1:?destination id required}
    x=${2:?x required}
    y=${3:?y required}
    target=${4:-$EXAMPLE_TARGET}
    run_atom interact drag --platform "$platform" --target "$target" --destination "$destination" --x "$x" --y "$y"
    ;;
  type-text)
    platform=ios
    case "${1:-}" in
      ios|android) platform=$1; shift ;;
    esac
    destination=${1:?destination id required}
    target_id=${2:?target id required}
    text=${3:?text required}
    target=${4:-$EXAMPLE_TARGET}
    run_atom interact type-text --platform "$platform" --target "$target" --destination "$destination" --target-id "$target_id" --text "$text"
    ;;
  evaluate)
    platform=ios
    case "${1:-}" in
      ios|android) platform=$1; shift ;;
    esac
    destination=${1:?destination id required}
    artifacts_dir=${2:?artifacts dir required}
    plan=${3:-$DEFAULT_PLAN}
    target=${4:-$EXAMPLE_TARGET}
    run_atom evaluate run --platform "$platform" --target "$target" --destination "$destination" --plan "$plan" --artifacts-dir "$artifacts_dir"
    ;;
  example-plan)
    printf '%s\n' "$DEFAULT_PLAN"
    ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "expected one of: tap, long-press, swipe, drag, type-text, evaluate, example-plan" >&2
    exit 64
    ;;
esac
