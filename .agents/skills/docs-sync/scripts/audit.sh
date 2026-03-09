#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../.." && pwd)
cd "$repo_root"

echo "==> Checking doc links"
for f in docs/*.md docs/**/*.md AGENTS.md README.md; do
  [ -f "$f" ] || continue
  # Extract markdown link targets (relative paths only)
  grep -oE '\]\([^)]+\)' "$f" 2>/dev/null | sed 's/\](//' | sed 's/)//' | while read -r target; do
    # Skip URLs and anchors
    if echo "$target" | grep -qE '^(http|#)'; then
      continue
    fi
    # Resolve relative to file's directory
    dir=$(dirname "$f")
    resolved="$dir/$target"
    if [ ! -e "$resolved" ]; then
      echo "  BROKEN: $f -> $target"
    fi
  done
done

echo ""
echo "==> Crate inventory vs architecture.md"
for crate_dir in crates/*/; do
  crate=$(basename "$crate_dir")
  if ! grep -q "$crate" docs/architecture.md 2>/dev/null; then
    echo "  MISSING from architecture.md: $crate"
  fi
done

echo ""
echo "==> Public trait/struct inventory"
for crate_dir in crates/*/; do
  crate=$(basename "$crate_dir")
  echo "  $crate:"
  grep -rn 'pub trait\|pub struct\|pub enum\|pub fn' "$crate_dir/src/" 2>/dev/null | \
    sed "s|$crate_dir||" | head -20
done

echo ""
echo "Audit complete."
