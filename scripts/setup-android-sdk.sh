#!/usr/bin/env sh
set -eu

# Installs the Android SDK, NDK, emulator, and a default AVD globally.
# Intended for local development — CI uses android-actions/setup-android instead.
#
# Also writes a .mise.local.toml so that ANDROID_HOME, ANDROID_NDK_HOME, and
# PATH are automatically set in future shells via mise.

repo_root=$(cd -- "$(dirname "$0")/.." && pwd)

ANDROID_HOME="${ANDROID_HOME:-$HOME/.android/sdk}"
ANDROID_NDK_HOME="$ANDROID_HOME/ndk/27.2.12479018"

if [ -d "$ANDROID_HOME/platforms/android-35" ] \
  && [ -x "$ANDROID_HOME/platform-tools/adb" ] \
  && [ -d "$ANDROID_HOME/system-images/android-35" ] \
  && [ -d "$ANDROID_HOME/ndk" ]; then
  echo "Android SDK already installed at $ANDROID_HOME"
  write_mise_local=true
else
  mkdir -p "$ANDROID_HOME"
  sdkmanager="$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager"

  if [ ! -x "$sdkmanager" ]; then
    os=$(uname -s | tr '[:upper:]' '[:lower:]')
    case "$os" in
      darwin) os="mac" ;;
      linux)  os="linux" ;;
      *)
        echo "unsupported OS: $os" >&2
        exit 1
        ;;
    esac

    zip_url="https://dl.google.com/android/repository/commandlinetools-${os}-12266719_latest.zip"
    tmp_zip=$(mktemp)
    echo "Downloading Android command-line tools..."
    curl -fsSL -o "$tmp_zip" "$zip_url"
    unzip -q -o "$tmp_zip" -d "$ANDROID_HOME"
    rm -f "$tmp_zip"

    # The zip extracts to cmdline-tools/ but sdkmanager expects cmdline-tools/latest/
    mv "$ANDROID_HOME/cmdline-tools" "$ANDROID_HOME/cmdline-tools-tmp"
    mkdir -p "$ANDROID_HOME/cmdline-tools"
    mv "$ANDROID_HOME/cmdline-tools-tmp" "$ANDROID_HOME/cmdline-tools/latest"
  fi

  echo "Accepting Android SDK licenses..."
  yes | "$sdkmanager" --licenses >/dev/null 2>&1 || true

  echo "Installing Android SDK packages..."
  "$sdkmanager" \
    "platforms;android-35" \
    "build-tools;35.0.0" \
    "platform-tools" \
    "emulator" \
    "ndk;27.2.12479018" \
    "system-images;android-35;google_apis;arm64-v8a" >/dev/null

  avdmanager="$ANDROID_HOME/cmdline-tools/latest/bin/avdmanager"
  if ! "$avdmanager" list avd 2>/dev/null | grep -q "atom_35"; then
    echo "Creating Android emulator AVD (atom_35)..."
    echo "no" | "$avdmanager" create avd \
      --name "atom_35" \
      --package "system-images;android-35;google_apis;arm64-v8a" \
      --device "pixel_6" \
      --force >/dev/null 2>&1
  fi

  echo "Android SDK installed at $ANDROID_HOME"
  write_mise_local=true
fi

if [ "${write_mise_local:-}" = "true" ]; then
  cat > "$repo_root/.mise.local.toml" <<EOF
[env]
ANDROID_HOME = "$ANDROID_HOME"
ANDROID_NDK_HOME = "$ANDROID_NDK_HOME"
_.path = ["$ANDROID_HOME/platform-tools", "$ANDROID_HOME/emulator"]
EOF
  echo "Wrote $repo_root/.mise.local.toml — Android env vars will be set automatically."
fi
