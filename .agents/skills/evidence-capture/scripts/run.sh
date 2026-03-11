#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

EXAMPLE_TARGET="//examples/hello-world/apps/hello_atom:hello_atom"

run_atom() {
  mise exec -- bazelisk run //:atom -- "$@"
}

mode=${1:-}

case "$mode" in
  screenshot)
    destination=${2:?destination id required}
    output=${3:?output path required}
    target=${4:-$EXAMPLE_TARGET}
    run_atom evidence screenshot --target "$target" --destination "$destination" --output "$output"
    ;;
  logs)
    destination=${2:?destination id required}
    output=${3:?output path required}
    seconds=${4:-60}
    target=${5:-$EXAMPLE_TARGET}
    run_atom evidence logs --target "$target" --destination "$destination" --output "$output" --seconds "$seconds"
    ;;
  video)
    destination=${2:?destination id required}
    output=${3:?output path required}
    seconds=${4:-5}
    target=${5:-$EXAMPLE_TARGET}
    run_atom evidence video --target "$target" --destination "$destination" --output "$output" --seconds "$seconds"
    ;;
  inspect-ui)
    destination=${2:?destination id required}
    output=${3:?output path required}
    target=${4:-$EXAMPLE_TARGET}
    run_atom inspect ui --target "$target" --destination "$destination" --output "$output"
    ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "expected one of: screenshot, logs, video, inspect-ui" >&2
    exit 64
    ;;
esac
