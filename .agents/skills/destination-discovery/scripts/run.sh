#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

run_atom() {
  mise exec -- bazelisk run //:atom -- "$@"
}

mode=${1:-all}

case "$mode" in
  all)
    run_atom destinations
    ;;
  all-json)
    run_atom destinations --json
    ;;
  ios)
    run_atom devices ios
    ;;
  ios-json)
    run_atom devices ios --json
    ;;
  android)
    run_atom devices android
    ;;
  android-json)
    run_atom devices android --json
    ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "expected one of: all, all-json, ios, ios-json, android, android-json" >&2
    exit 64
    ;;
esac
