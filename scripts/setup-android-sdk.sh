#!/usr/bin/env sh
set -eu

# Installs the Android SDK command-line tools and accepts licenses.
# Requires ANDROID_HOME to be set (mise.toml sets this automatically).

if [ -z "${ANDROID_HOME:-}" ]; then
  echo "ANDROID_HOME is not set" >&2
  exit 1
fi

if [ -d "$ANDROID_HOME/platforms/android-35" ]; then
  echo "Android SDK already installed at $ANDROID_HOME"
  exit 0
fi

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

echo "Installing Android SDK platform and build tools..."
"$sdkmanager" "platforms;android-35" "build-tools;35.0.0" >/dev/null

echo "Android SDK installed at $ANDROID_HOME"
