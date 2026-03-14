#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

EXAMPLE_TARGET="//examples/hello-world/apps/hello_atom:hello_atom"

run_atom() {
  mise exec -- bazelisk run //:atom -- "$@"
}

mode=${1:-}
shift || true

case "$mode" in
  screenshot)
    platform=ios
    case "${1:-}" in
      ios|android) platform=$1; shift ;;
    esac
    destination=${1:?destination id required}
    output=${2:?output path required}
    target=${3:-$EXAMPLE_TARGET}
    run_atom evidence screenshot --platform "$platform" --target "$target" --destination "$destination" --output "$output"
    ;;
  logs)
    platform=ios
    case "${1:-}" in
      ios|android) platform=$1; shift ;;
    esac
    destination=${1:?destination id required}
    output=${2:?output path required}
    seconds=${3:-60}
    target=${4:-$EXAMPLE_TARGET}
    run_atom evidence logs --platform "$platform" --target "$target" --destination "$destination" --output "$output" --seconds "$seconds"
    ;;
  video)
    platform=ios
    case "${1:-}" in
      ios|android) platform=$1; shift ;;
    esac
    destination=${1:?destination id required}
    output=${2:?output path required}
    seconds=${3:-5}
    target=${4:-$EXAMPLE_TARGET}
    run_atom evidence video --platform "$platform" --target "$target" --destination "$destination" --output "$output" --seconds "$seconds"
    ;;
  inspect-ui)
    platform=ios
    case "${1:-}" in
      ios|android) platform=$1; shift ;;
    esac
    destination=${1:?destination id required}
    output=${2:?output path required}
    target=${3:-$EXAMPLE_TARGET}
    run_atom inspect ui --platform "$platform" --target "$target" --destination "$destination" --output "$output"
    ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "expected one of: screenshot, logs, video, inspect-ui" >&2
    exit 64
    ;;
esac
