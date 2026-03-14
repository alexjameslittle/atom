#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)
cd "$repo_root"

if [ -z "${JAVA_HOME:-}" ] && command -v mise >/dev/null 2>&1; then
  java_home=$(mise where java 2>/dev/null || true)
  if [ -n "$java_home" ]; then
    JAVA_HOME=$java_home
    export JAVA_HOME
    PATH="$JAVA_HOME/bin:$PATH"
    export PATH
  fi
fi

tmp_parent=${ATOM_VERIFY_TMP_ROOT:-${TMPDIR:-/tmp}}
mkdir -p "$tmp_parent"
scratch_root=$(mktemp -d "$tmp_parent/atom-scaffold.XXXXXX")

project_name=ci_test_app
project_root="$scratch_root/$project_name"
atom_bin="$repo_root/bazel-bin/crates/atom-cli/atom"
plan_path="$scratch_root/$project_name.prebuild.bin"

trap '(cd "$project_root" 2>/dev/null && bazelisk shutdown 2>/dev/null) || true; rm -rf "$scratch_root"' EXIT INT TERM

bazelisk build //:atom >/dev/null
(cd "$scratch_root" && "$atom_bin" new "$project_name" >/dev/null)

escaped_repo_root=$(printf '%s\n' "$repo_root" | sed 's/[\/&]/\\&/g')
# Validate the current checkout, not the published main branch baked into the scaffold template.
perl -0pi -e 's|git_override\(\n    module_name = "atom",\n    remote = ".*?",\n    branch = ".*?",\n\)|local_path_override(\n    module_name = "atom",\n    path = "'"$escaped_repo_root"'"\n)|s' "$project_root/MODULE.bazel"

(cd "$project_root" && "$atom_bin" prebuild --target "//apps/$project_name:$project_name" --dry-run >"$plan_path")

test -s "$plan_path"

expect_plan_contains() {
  pattern=$1
  if ! grep -aF -- "$pattern" "$plan_path" >/dev/null; then
    echo "ERROR: expected scaffolded prebuild plan to contain: $pattern" >&2
    exit 1
  fi
}

expect_plan_contains "generated/ios/ci-test-app"
expect_plan_contains "//generated/ios/ci-test-app:app"
expect_plan_contains "generated/android/ci-test-app"
expect_plan_contains "//generated/android/ci-test-app:app"
