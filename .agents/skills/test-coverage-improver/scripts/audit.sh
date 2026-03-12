#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/../../../.." && pwd)
cd "$repo_root"

detect_base_ref() {
  for candidate in "${TEST_COVERAGE_BASE_REF:-}" origin/main main; do
    [ -n "$candidate" ] || continue
    if git rev-parse --verify "$candidate" >/dev/null 2>&1; then
      echo "$candidate"
      return 0
    fi
  done
  return 1
}

collect_changed_files() {
  if base_ref=$(detect_base_ref); then
    merge_base=$(git merge-base HEAD "$base_ref")
    git diff --name-only "$merge_base"..HEAD
  fi
  git diff --name-only
  git diff --cached --name-only
  git ls-files --others --exclude-standard
}

extract_rust_test_targets() {
  build_file="$1"
  crate_label="$2"

  awk -v crate_label="$crate_label" '
    /rust_test\(/ { in_test = 1; next }
    in_test && /name = "/ {
      line = $0
      sub(/.*name = "/, "", line)
      sub(/".*/, "", line)
      print "    " crate_label ":" line
      in_test = 0
      next
    }
    in_test && /^\)/ { in_test = 0 }
  ' "$build_file"
}

changed_files=$(collect_changed_files | awk 'NF' | sort -u)

if [ -z "$changed_files" ]; then
  echo "No changed files detected."
  exit 0
fi

echo "==> Changed files"
printf '%s\n' "$changed_files" | sed 's#^#  #'

echo ""
echo "==> Nearby test targets"
for path in $changed_files; do
  case "$path" in
    crates/*)
      crate=$(printf '%s' "$path" | cut -d/ -f2)
      build_file="crates/$crate/BUILD.bazel"
      echo "  $path"
      if [ -f "$build_file" ]; then
        extract_rust_test_targets "$build_file" "//crates/$crate" || true
      else
        echo "    (no BUILD.bazel found)"
      fi
      find "crates/$crate/tests" -type f 2>/dev/null | sort | sed 's#^#    #' || true
      ;;
    examples/*)
      echo "  $path"
      printf '%s\n' \
        "    example validation: mise exec -- ./.agents/skills/examples-auto-run/scripts/run.sh smoke" \
        "    example validation: mise exec -- ./.agents/skills/examples-auto-run/scripts/run.sh generated-tree"
      ;;
    bzl/*|scripts/*|.agents/skills/*)
      echo "  $path"
      printf '%s\n' \
        "    verification target: mise exec -- ./scripts/verify.sh lint" \
        "    smoke target: mise exec -- bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run"
      ;;
  esac
done

echo ""
echo "==> Files with no obvious nearby tests"
for path in $changed_files; do
  case "$path" in
    crates/*)
      crate=$(printf '%s' "$path" | cut -d/ -f2)
      build_file="crates/$crate/BUILD.bazel"
      if [ ! -f "$build_file" ] || ! grep -q 'rust_test(' "$build_file"; then
        echo "  $path"
      fi
      ;;
    examples/*|bzl/*|scripts/*|.agents/skills/*)
      echo "  $path"
      ;;
  esac
done
