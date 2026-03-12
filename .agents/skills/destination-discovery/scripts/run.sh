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
    printf '==> iOS\n'
    run_atom destinations --platform ios
    printf '\n==> Android\n'
    run_atom destinations --platform android
    ;;
  all-json)
    printf '{\n  "ios": '
    run_atom destinations --platform ios --json
    printf ',\n  "android": '
    run_atom destinations --platform android --json
    printf '\n}\n'
    ;;
  ios)
    run_atom devices --platform ios
    ;;
  ios-json)
    run_atom devices --platform ios --json
    ;;
  android)
    run_atom devices --platform android
    ;;
  android-json)
    run_atom devices --platform android --json
    ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "expected one of: all, all-json, ios, ios-json, android, android-json" >&2
    exit 64
    ;;
esac
