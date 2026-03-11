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

case "$mode" in
  tap)
    destination=${2:?destination id required}
    target_id=${3:?target id required}
    target=${4:-$EXAMPLE_TARGET}
    run_atom interact tap --target "$target" --destination "$destination" --target-id "$target_id"
    ;;
  long-press)
    destination=${2:?destination id required}
    target_id=${3:?target id required}
    target=${4:-$EXAMPLE_TARGET}
    run_atom interact long-press --target "$target" --destination "$destination" --target-id "$target_id"
    ;;
  swipe)
    destination=${2:?destination id required}
    x=${3:?x required}
    y=${4:?y required}
    target=${5:-$EXAMPLE_TARGET}
    run_atom interact swipe --target "$target" --destination "$destination" --x "$x" --y "$y"
    ;;
  drag)
    destination=${2:?destination id required}
    x=${3:?x required}
    y=${4:?y required}
    target=${5:-$EXAMPLE_TARGET}
    run_atom interact drag --target "$target" --destination "$destination" --x "$x" --y "$y"
    ;;
  type-text)
    destination=${2:?destination id required}
    target_id=${3:?target id required}
    text=${4:?text required}
    target=${5:-$EXAMPLE_TARGET}
    run_atom interact type-text --target "$target" --destination "$destination" --target-id "$target_id" --text "$text"
    ;;
  evaluate)
    destination=${2:?destination id required}
    artifacts_dir=${3:?artifacts dir required}
    plan=${4:-$DEFAULT_PLAN}
    target=${5:-$EXAMPLE_TARGET}
    run_atom evaluate run --target "$target" --destination "$destination" --plan "$plan" --artifacts-dir "$artifacts_dir"
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
