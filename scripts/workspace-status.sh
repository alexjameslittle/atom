#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)

read_mise_tool_version() {
  key=$1
  awk -v key="$key" '
    $0 ~ "^[[:space:]]*" key "[[:space:]]*=[[:space:]]*\"" {
      split($0, parts, "\"")
      print parts[2]
      exit
    }
  ' "$repo_root/mise.toml"
}

read_module_dep_version() {
  key=$1
  awk -v key="$key" '
    $0 ~ "bazel_dep\\(name = \"" key "\", version = \"" {
      split($0, parts, "\"")
      print parts[4]
      exit
    }
  ' "$repo_root/MODULE.bazel"
}

read_bazelrc_setting() {
  prefix=$1
  awk -v prefix="$prefix" '
    index($0, prefix) == 1 {
      split($0, parts, "=")
      print parts[2]
      exit
    }
  ' "$repo_root/.bazelrc"
}

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

bazelisk_version=$(read_mise_tool_version bazelisk)
if [ -z "${bazelisk_version:-}" ]; then
  bazelisk_version="unknown"
fi

rust_toolchain_version=$(read_mise_tool_version rust)
if [ -z "${rust_toolchain_version:-}" ]; then
  rust_toolchain_version="unknown"
fi

java_version=$(read_mise_tool_version java)
if [ -z "${java_version:-}" ]; then
  java_version="unknown"
fi

rules_rust_version=$(read_module_dep_version rules_rust)
if [ -z "${rules_rust_version:-}" ]; then
  rules_rust_version="unknown"
fi

java_runtime_version=$(read_bazelrc_setting "build --java_runtime_version=")
if [ -z "${java_runtime_version:-}" ]; then
  java_runtime_version="unknown"
fi

printf 'STABLE_ATOM_FRAMEWORK_VERSION %s\n' "$atom_version"
printf 'STABLE_ATOM_RUST_VERSION %s\n' "$rust_version"
printf 'STABLE_ATOM_BUILD_BAZEL_VERSION %s\n' "$build_bazel_version"
printf 'STABLE_ATOM_MISE_BAZELISK_VERSION %s\n' "$bazelisk_version"
printf 'STABLE_ATOM_MISE_RUST_TOOLCHAIN_VERSION %s\n' "$rust_toolchain_version"
printf 'STABLE_ATOM_MISE_JAVA_VERSION %s\n' "$java_version"
printf 'STABLE_ATOM_RULES_RUST_VERSION %s\n' "$rules_rust_version"
printf 'STABLE_ATOM_JAVA_RUNTIME_VERSION %s\n' "$java_runtime_version"
