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
ANDROID_SYSTEM_IMAGE_PACKAGE="system-images;android-35;default;arm64-v8a"
ANDROID_SYSTEM_IMAGE_DIR="$ANDROID_HOME/system-images/android-35/default/arm64-v8a"
ATOM_AVD_NAME="atom_35"

desired_atom_avd_config() {
  config="$HOME/.android/avd/$ATOM_AVD_NAME.avd/config.ini"
  [ -f "$config" ] || return 1
  grep -q '^image.sysdir.1 = system-images/android-35/default/arm64-v8a/$' "$config"
}

set_avd_config_value() {
  config="$1"
  key="$2"
  value="$3"
  tmp=$(mktemp)
  awk -v key="$key" -v value="$value" '
    BEGIN { updated = 0 }
    index($0, key " = ") == 1 {
      print key " = " value
      updated = 1
      next
    }
    { print }
    END {
      if (!updated) {
        print key " = " value
      }
    }
  ' "$config" > "$tmp"
  mv "$tmp" "$config"
}

normalize_atom_avd_appearance() {
  config="$HOME/.android/avd/$ATOM_AVD_NAME.avd/config.ini"
  [ -f "$config" ] || return 0
  set_avd_config_value "$config" "showDeviceFrame" "no"
  set_avd_config_value "$config" "hw.gpu.enabled" "yes"
  set_avd_config_value "$config" "hw.gpu.mode" "host"
}

if [ -d "$ANDROID_HOME/platforms/android-35" ] \
  && [ -x "$ANDROID_HOME/platform-tools/adb" ] \
  && [ -d "$ANDROID_SYSTEM_IMAGE_DIR" ] \
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
    "$ANDROID_SYSTEM_IMAGE_PACKAGE" >/dev/null

  echo "Android SDK installed at $ANDROID_HOME"
  write_mise_local=true
fi

avdmanager="$ANDROID_HOME/cmdline-tools/latest/bin/avdmanager"
if desired_atom_avd_config; then
  echo "Android emulator AVD ($ATOM_AVD_NAME) already uses the default system image"
else
  if "$avdmanager" list avd 2>/dev/null | grep -q "$ATOM_AVD_NAME"; then
    echo "Recreating Android emulator AVD ($ATOM_AVD_NAME) with the default system image..."
    "$avdmanager" delete avd --name "$ATOM_AVD_NAME" >/dev/null 2>&1 || true
    rm -rf "$HOME/.android/avd/$ATOM_AVD_NAME.avd" "$HOME/.android/avd/$ATOM_AVD_NAME.ini"
  else
    echo "Creating Android emulator AVD ($ATOM_AVD_NAME)..."
  fi

  echo "no" | "$avdmanager" create avd \
    --name "$ATOM_AVD_NAME" \
    --package "$ANDROID_SYSTEM_IMAGE_PACKAGE" \
    --device "pixel_6" \
    --force >/dev/null 2>&1
fi

normalize_atom_avd_appearance

if [ "${write_mise_local:-}" = "true" ]; then
  cat > "$repo_root/.mise.local.toml" <<EOF
[env]
ANDROID_HOME = "$ANDROID_HOME"
ANDROID_NDK_HOME = "$ANDROID_NDK_HOME"
_.path = ["$ANDROID_HOME/platform-tools", "$ANDROID_HOME/emulator"]
EOF
  echo "Wrote $repo_root/.mise.local.toml — Android env vars will be set automatically."
fi
