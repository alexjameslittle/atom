#!/usr/bin/env sh
set -eu

version="0.7.21"

if ! command -v npm >/dev/null 2>&1; then
  echo "npm is required to install agent-device; ensure the pinned Node.js toolchain is available" >&2
  exit 69
fi

current_version=""
if command -v agent-device >/dev/null 2>&1; then
  current_version=$(agent-device --version 2>/dev/null || true)
fi

if [ "$current_version" = "$version" ]; then
  exit 0
fi

npm install --global "agent-device@$version"

if ! command -v agent-device >/dev/null 2>&1; then
  echo "agent-device install failed: expected the agent-device CLI to be available on PATH" >&2
  exit 69
fi

installed_version=$(agent-device --version 2>/dev/null || true)
if [ "$installed_version" != "$version" ]; then
  echo "agent-device install failed: expected version $version but found $installed_version" >&2
  exit 69
fi
