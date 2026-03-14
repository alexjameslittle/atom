#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)

atom_version=$(awk -F'"' '/^[[:space:]]*version[[:space:]]*=[[:space:]]*"/ { print $2; exit }' "$repo_root/MODULE.bazel")
if [ -z "${atom_version:-}" ]; then
  atom_version="unknown"
fi

build_bazel_version=$(awk 'NF { print $1; exit }' "$repo_root/.bazelversion" 2>/dev/null || true)
if [ -z "${build_bazel_version:-}" ]; then
  build_bazel_version="unknown"
fi

rust_version="unknown"
if command -v rustc >/dev/null 2>&1; then
  rust_version=$(rustc --version 2>/dev/null | awk 'NF { print $2; exit }' || true)
  if [ -z "${rust_version:-}" ]; then
    rust_version="unknown"
  fi
fi

printf 'STABLE_ATOM_FRAMEWORK_VERSION %s\n' "$atom_version"
printf 'STABLE_ATOM_RUST_VERSION %s\n' "$rust_version"
printf 'STABLE_ATOM_BUILD_BAZEL_VERSION %s\n' "$build_bazel_version"
