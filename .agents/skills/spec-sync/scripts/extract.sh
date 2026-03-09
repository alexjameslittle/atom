#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../.." && pwd)
cd "$repo_root"

echo "==> Error codes from atom-ffi"
grep -n 'AtomErrorCode\|error_code\|exit_code' crates/atom-ffi/src/*.rs 2>/dev/null || echo "  (none found)"

echo ""
echo "==> Lifecycle states from atom-runtime"
grep -n 'RuntimeState\|LifecycleEvent\|AtomLifecycleEvent' crates/atom-runtime/src/*.rs 2>/dev/null || echo "  (none found)"

echo ""
echo "==> Module metadata fields from atom-modules"
grep -n 'pub.*:' crates/atom-modules/src/*.rs 2>/dev/null | grep -i 'struct\|field\|manifest' || echo "  (none found)"

echo ""
echo "==> Exit codes from atom-cli"
grep -n 'exit\|process::exit\|ExitCode\|exit_code' crates/atom-cli/src/*.rs 2>/dev/null || echo "  (none found)"

echo ""
echo "==> Corresponding SPEC.md sections"
grep -n '## \|### ' SPEC.md | head -40

echo ""
echo "Extraction complete. Compare the above against SPEC.md content."
