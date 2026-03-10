#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

EXAMPLE_TARGET="//examples/hello-world/apps/hello_atom:hello_atom"
mode=${1:-smoke}

echo "==> examples-auto-run mode: $mode"

case "$mode" in
  smoke)
    bazelisk run //:atom -- prebuild --target "$EXAMPLE_TARGET" --dry-run
    ;;
  generated-tree)
    bazelisk run //:atom -- prebuild --target "$EXAMPLE_TARGET"
    if [ -d generated ]; then
      find generated -maxdepth 3 -type f | sort | sed 's#^#  #'
    else
      echo "  generated/ not found"
    fi
    ;;
  android)
    ./scripts/verify.sh build-android
    ;;
  ios)
    ./scripts/verify.sh build-ios
    ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "expected one of: smoke, generated-tree, android, ios" >&2
    exit 64
    ;;
esac
