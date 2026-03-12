#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

EXAMPLE_TARGET="//examples/hello-world/apps/hello_atom:hello_atom"

run_atom() {
  mise exec -- bazelisk run //:atom -- "$@"
}

detect_generated_root() {
  for candidate in "$repo_root/cng-output" "$repo_root/generated"; do
    if [ -d "$candidate/ios" ] || [ -d "$candidate/android" ] || [ -d "$candidate/schema" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  printf '%s\n' "$repo_root/cng-output"
}

echo "==> Running dry-run prebuild"
run_atom prebuild --target "$EXAMPLE_TARGET" --dry-run

echo ""
echo "==> Running real prebuild"
run_atom prebuild --target "$EXAMPLE_TARGET"

GENERATED_ROOT=$(detect_generated_root)

echo ""
echo "==> Using generated root"
echo "  ${GENERATED_ROOT#"$repo_root"/}"

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
run_atom prebuild --target "$EXAMPLE_TARGET"
hash_after=$(find "$GENERATED_ROOT" -type f -exec shasum {} \; 2>/dev/null | sort | shasum)

if [ "$hash_before" = "$hash_after" ]; then
  echo "  PASS: Output is deterministic"
else
  echo "  FAIL: Output differs between runs"
  exit 1
fi

echo ""
echo "Prebuild validation complete."
