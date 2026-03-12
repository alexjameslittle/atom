#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

run_mise() {
  mise exec -- "$@"
}

echo "==> Extracting crate dependencies from Bazel"
for crate_dir in crates/*/; do
  crate=$(basename "$crate_dir")
  label="//crates/$crate:$crate"
  echo ""
  echo "  $crate depends on:"
  # List direct Rust library deps within //crates/
  run_mise bazelisk query "filter('//crates/', deps($label, 1))" 2>/dev/null | \
    grep -v "^$label$" | sed 's/^/    /' || echo "    (none or query failed)"
done

echo ""
echo "==> Checking for reverse dependencies"

# atom-runtime should not depend on CLI/CNG/deploy crates
runtime_deps=$(run_mise bazelisk query "deps(//crates/atom-runtime:atom-runtime)" 2>/dev/null || echo "")
for forbidden in atom-cli atom-cng atom-deploy; do
  if echo "$runtime_deps" | grep -q "//crates/$forbidden"; then
    echo "  VIOLATION: atom-runtime depends on $forbidden"
  fi
done

echo ""
run_mise sh scripts/check-generic-backend-leaks.sh

echo ""
echo "Dependency check complete."
