#!/usr/bin/env sh
set -eu

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)
cd "$repo_root"

mode=${1:-verify}

# Source directories to verify. Generated host trees are excluded because their
# BUILD files reference platform-specific rules (android_binary, UIKit) that are
# only valid when built with the correct platform flags via `atom run`.
# If you add a new top-level directory with BUILD files, add it here.
VERIFY_PACKAGES="//crates/... //examples/... //bzl/... //tools/... //platforms/..."

check_for_unverified_packages() {
  for dir in "$repo_root"/*/; do
    name=$(basename "$dir")
    case "$name" in
      generated|cng-output|bazel-*|docs|scripts|templates|node_modules) continue ;;
    esac
    if [ -f "$dir/BUILD.bazel" ] || [ -f "$dir/BUILD" ]; then
      if ! echo "$VERIFY_PACKAGES" | grep -q "//${name}/\.\.\."; then
        echo "ERROR: //${name}/... has BUILD files but is not in VERIFY_PACKAGES in scripts/verify.sh" >&2
        exit 1
      fi
    fi
  done
}

lint() {
  check_for_unverified_packages
  sh scripts/check-generic-backend-leaks.sh
  # shellcheck disable=SC2086
  mise exec -- bazelisk build --config=lint --@aspect_rules_lint//lint:fail_on_violation --keep_going $VERIFY_PACKAGES
  mise exec -- bazelisk run //:format.check
  mise exec -- shellcheck .githooks/pre-commit .githooks/pre-push .mise/tasks/* scripts/*.sh .agents/skills/*/scripts/*.sh ./install.sh tools/install/*.sh
  mise exec -- actionlint
}

test_suite() {
  # shellcheck disable=SC2086
  mise exec -- bazelisk test $VERIFY_PACKAGES
  mise exec -- bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run >/dev/null
  sh scripts/verify-scaffold-project.sh
}

EXAMPLE_TARGET="//examples/hello-world/apps/hello_atom:hello_atom"

generate_example_app() {
  # Generate BUILD files for the example app (non-dry-run).
  mise exec -- bazelisk run //:atom -- prebuild --target "$EXAMPLE_TARGET"
}

build_ios_app() {
  # Build iOS app (simulator architecture).
  mise exec -- bazelisk build //cng-output/ios/hello-atom:app --ios_multi_cpus=sim_arm64
}

build_android_app() {
  # Build Android app (requires ANDROID_HOME).
  if [ -n "${ANDROID_HOME:-}" ]; then
    mise exec -- bazelisk build //cng-output/android/hello-atom:app --android_platforms=//platforms:arm64-v8a
  else
    echo "ANDROID_HOME not set, skipping Android build"
  fi
}

build_apps() {
  generate_example_app
  build_ios_app
  build_android_app
}

case "$mode" in
  lint)
    lint
    ;;
  test)
    test_suite
    ;;
  build)
    build_apps
    ;;
  build-ios)
    generate_example_app
    build_ios_app
    ;;
  build-android)
    generate_example_app
    build_android_app
    ;;
  pre-push)
    lint
    test_suite
    ;;
  verify)
    lint
    test_suite
    build_apps
    ;;
  *)
    echo "unknown verify mode: $mode" >&2
    exit 64
    ;;
esac
