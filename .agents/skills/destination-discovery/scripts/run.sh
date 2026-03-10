#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

mode=${1:-all}

case "$mode" in
  all)
    bazelisk run //:atom -- destinations
    ;;
  all-json)
    bazelisk run //:atom -- destinations --json
    ;;
  ios)
    bazelisk run //:atom -- devices ios
    ;;
  ios-json)
    bazelisk run //:atom -- devices ios --json
    ;;
  android)
    bazelisk run //:atom -- devices android
    ;;
  android-json)
    bazelisk run //:atom -- devices android --json
    ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "expected one of: all, all-json, ios, ios-json, android, android-json" >&2
    exit 64
    ;;
esac
