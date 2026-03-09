#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../.." && pwd)
cd "$repo_root"

EXAMPLE_TARGET="//examples/hello-world/apps/hello_atom:hello_atom"
GENERATED_ROOT="$repo_root/generated"

echo "==> Running dry-run prebuild"
bazelisk run //:atom -- prebuild --target "$EXAMPLE_TARGET" --dry-run

echo ""
echo "==> Running real prebuild"
bazelisk run //:atom -- prebuild --target "$EXAMPLE_TARGET"

echo ""
echo "==> Checking generated tree structure"
if [ -d "$GENERATED_ROOT/ios" ]; then
  echo "  iOS tree present"
  find "$GENERATED_ROOT/ios" -type f | head -20 | sed 's/^/    /'
else
  echo "  WARNING: No iOS generated tree"
fi

if [ -d "$GENERATED_ROOT/android" ]; then
  echo "  Android tree present"
  find "$GENERATED_ROOT/android" -type f | head -20 | sed 's/^/    /'
else
  echo "  WARNING: No Android generated tree"
fi

echo ""
echo "==> Checking determinism (second run)"
hash_before=$(find "$GENERATED_ROOT" -type f -exec shasum {} \; 2>/dev/null | sort | shasum)
bazelisk run //:atom -- prebuild --target "$EXAMPLE_TARGET"
hash_after=$(find "$GENERATED_ROOT" -type f -exec shasum {} \; 2>/dev/null | sort | shasum)

if [ "$hash_before" = "$hash_after" ]; then
  echo "  PASS: Output is deterministic"
else
  echo "  FAIL: Output differs between runs"
  exit 1
fi

echo ""
echo "Prebuild validation complete."
