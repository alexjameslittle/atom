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
    platform=${2:?platform required}
    destination=${3:?destination id required}
    output=${4:?output path required}
    target=${5:-$EXAMPLE_TARGET}
    run_atom evidence screenshot --platform "$platform" --target "$target" --destination "$destination" --output "$output"
    ;;
  logs)
    platform=${2:?platform required}
    destination=${3:?destination id required}
    output=${4:?output path required}
    seconds=${5:-60}
    target=${6:-$EXAMPLE_TARGET}
    run_atom evidence logs --platform "$platform" --target "$target" --destination "$destination" --output "$output" --seconds "$seconds"
    ;;
  video)
    platform=${2:?platform required}
    destination=${3:?destination id required}
    output=${4:?output path required}
    seconds=${5:-5}
    target=${6:-$EXAMPLE_TARGET}
    run_atom evidence video --platform "$platform" --target "$target" --destination "$destination" --output "$output" --seconds "$seconds"
    ;;
  inspect-ui)
    platform=${2:?platform required}
    destination=${3:?destination id required}
    output=${4:?output path required}
    target=${5:-$EXAMPLE_TARGET}
    run_atom inspect ui --platform "$platform" --target "$target" --destination "$destination" --output "$output"
    ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "expected one of: screenshot, logs, video, inspect-ui" >&2
    exit 64
    ;;
esac
