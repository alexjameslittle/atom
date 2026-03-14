#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

run_atom() {
  mise exec -- bazelisk run //:atom -- "$@"
}

merge_json() {
  ios_path=$1
  android_path=$2
  python3 - "$ios_path" "$android_path" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as ios_file:
    ios = json.load(ios_file)
with open(sys.argv[2], encoding="utf-8") as android_file:
    android = json.load(android_file)

print(json.dumps(ios + android, indent=2))
PY
}

mode=${1:-all}

case "$mode" in
  all)
    printf '==> ios\n'
    run_atom destinations --platform ios
    printf '\n==> android\n'
    run_atom destinations --platform android
    ;;
  all-json)
    ios_json=$(mktemp)
    android_json=$(mktemp)
    trap 'rm -f "$ios_json" "$android_json"' EXIT HUP INT TERM
    run_atom destinations --platform ios --json >"$ios_json"
    run_atom destinations --platform android --json >"$android_json"
    merge_json "$ios_json" "$android_json"
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
