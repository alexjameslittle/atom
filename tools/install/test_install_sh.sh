#!/usr/bin/env sh
set -eu

# shellcheck disable=SC2154
INSTALLER="${TEST_SRCDIR}/${TEST_WORKSPACE}/install.sh"
SYSTEM_PATH=$PATH

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

assert_contains() {
  haystack=$1
  needle=$2
  printf '%s' "$haystack" | grep -F "$needle" >/dev/null 2>&1 || fail "expected to find '$needle'"
}

assert_equals() {
  expected=$1
  actual=$2
  [ "$expected" = "$actual" ] || fail "expected '$expected', got '$actual'"
}

assert_file_exists() {
  [ -f "$1" ] || fail "expected file '$1' to exist"
}

assert_exit_code() {
  expected=$1
  actual=$2
  [ "$expected" -eq "$actual" ] || fail "expected exit code $expected, got $actual"
}

compute_sha256() {
  file=$1

  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
    return 0
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
    return 0
  fi

  fail "missing shasum and sha256sum in test environment"
}

write_release_asset() {
  version=$1
  asset_dir=$2
  asset_path=$asset_dir/atom-darwin-arm64
  checksum_path=$asset_dir/atom-darwin-arm64.sha256

  mkdir -p "$asset_dir"
  cat >"$asset_path" <<EOF
#!/usr/bin/env sh
if [ "\${1:-}" = "--help" ]; then
  printf 'atom ${version} help\n'
else
  printf 'atom ${version}\n'
fi
EOF
  chmod 755 "$asset_path"

  checksum=$(compute_sha256 "$asset_path")
  printf '%s  atom-darwin-arm64\n' "$checksum" >"$checksum_path"
}

write_bad_release_asset() {
  version=$1
  asset_dir=$2
  asset_path=$asset_dir/atom-darwin-arm64
  checksum_path=$asset_dir/atom-darwin-arm64.sha256

  write_release_asset "$version" "$asset_dir"
  printf 'badbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadb  atom-darwin-arm64\n' >"$checksum_path"
}

write_release_index() {
  api_dir=$1
  mkdir -p "$api_dir"
  cat >"$api_dir/releases.json" <<'EOF'
[
  {
    "tag_name": "v0.2.0",
    "prerelease": true
  },
  {
    "tag_name": "v0.1.0",
    "prerelease": true
  }
]
EOF
}

write_stub_curl() {
  stub_path=$1/curl
  cat >"$stub_path" <<'EOF'
#!/usr/bin/env sh
set -eu

out=
url=

while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      out=$2
      shift 2
      ;;
    -f|-s|-S|-L|-fsSL)
      shift
      ;;
    *)
      url=$1
      shift
      ;;
  esac
done

case "$url" in
  file://*)
    path=${url#file://}
    ;;
  *)
    printf 'unsupported url: %s\n' "$url" >&2
    exit 1
    ;;
esac

if [ -n "$out" ]; then
  cp "$path" "$out"
else
  cat "$path"
fi
EOF
  chmod 755 "$stub_path"
}

write_stub_uname() {
  stub_path=$1/uname
  cat >"$stub_path" <<'EOF'
#!/usr/bin/env sh
set -eu

case "${1:-}" in
  -s)
    printf '%s\n' "${FAKE_UNAME_S:-Darwin}"
    ;;
  -m)
    printf '%s\n' "${FAKE_UNAME_M:-arm64}"
    ;;
  *)
    printf 'unsupported uname invocation\n' >&2
    exit 1
    ;;
esac
EOF
  chmod 755 "$stub_path"
}

run_installer() {
  home_dir=$1
  shift
  PATH="$STUBS:$SYSTEM_PATH" \
    HOME="$home_dir" \
    ATOM_INSTALL_RELEASES_API="file://${API_DIR}/releases.json" \
    ATOM_INSTALL_DOWNLOAD_BASE="file://${DOWNLOAD_DIR}" \
    /bin/sh "$INSTALLER" "$@"
}

run_with_fake_platform() {
  home_dir=$1
  fake_os=$2
  fake_arch=$3
  shift 3
  PATH="$STUBS:$SYSTEM_PATH" \
    HOME="$home_dir" \
    FAKE_UNAME_S="$fake_os" \
    FAKE_UNAME_M="$fake_arch" \
    ATOM_INSTALL_RELEASES_API="file://${API_DIR}/releases.json" \
    ATOM_INSTALL_DOWNLOAD_BASE="file://${DOWNLOAD_DIR}" \
    /bin/sh "$INSTALLER" "$@"
}

assert_help_version() {
  home_dir=$1
  expected=$2
  output=$(PATH="/usr/bin:/bin" HOME="$home_dir" /bin/sh -c '. "$HOME/.zshrc"; atom --help')
  assert_contains "$output" "atom ${expected} help"
}

ROOT=$(mktemp -d "${TMPDIR:-/tmp}/atom-install-test.XXXXXX")
trap 'rm -rf "$ROOT"' EXIT HUP INT TERM

API_DIR=$ROOT/api
DOWNLOAD_DIR=$ROOT/downloads
STUBS=$ROOT/stubs

mkdir -p "$STUBS"
write_stub_curl "$STUBS"
write_stub_uname "$STUBS"
write_release_index "$API_DIR"
write_release_asset "v0.1.0" "$DOWNLOAD_DIR/v0.1.0"
write_release_asset "v0.2.0" "$DOWNLOAD_DIR/v0.2.0"
write_bad_release_asset "v9.9.9" "$DOWNLOAD_DIR/v9.9.9"

home_latest=$ROOT/home-latest
mkdir -p "$home_latest"
: >"$home_latest/.zshrc"

latest_output=$(run_installer "$home_latest")
assert_contains "$latest_output" "Atom v0.2.0 installed to $home_latest/.atom/bin/atom"
assert_contains "$(cat "$home_latest/.zshrc")" "export PATH=\"$home_latest/.atom/bin:\$PATH\""
assert_help_version "$home_latest" "v0.2.0"

home_upgrade=$ROOT/home-upgrade
mkdir -p "$home_upgrade"
: >"$home_upgrade/.zshrc"

run_installer "$home_upgrade" --version v0.1.0 >/dev/null
assert_help_version "$home_upgrade" "v0.1.0"
run_installer "$home_upgrade" >/dev/null
assert_help_version "$home_upgrade" "v0.2.0"
profile_line_count=$(grep -F -c "export PATH=\"$home_upgrade/.atom/bin:\$PATH\"" "$home_upgrade/.zshrc")
assert_equals "1" "$profile_line_count"

home_prefix=$ROOT/home-prefix
custom_prefix=$home_prefix/custom-root
mkdir -p "$home_prefix"
: >"$home_prefix/.bashrc"

run_installer "$home_prefix" --version v0.1.0 --prefix "$custom_prefix" >/dev/null
assert_file_exists "$custom_prefix/bin/atom"
assert_contains "$(cat "$home_prefix/.bashrc")" "export PATH=\"$custom_prefix/bin:\$PATH\""

home_checksum=$ROOT/home-checksum
mkdir -p "$home_checksum"
: >"$home_checksum/.zshrc"

run_installer "$home_checksum" --version v0.1.0 >/dev/null
set +e
checksum_output=$(run_installer "$home_checksum" --version v9.9.9 2>&1)
checksum_status=$?
set -e
assert_exit_code 1 "$checksum_status"
assert_contains "$checksum_output" "SHA256 checksum mismatch"
assert_help_version "$home_checksum" "v0.1.0"

home_unsupported=$ROOT/home-unsupported
mkdir -p "$home_unsupported"
: >"$home_unsupported/.zshrc"

set +e
linux_output=$(run_with_fake_platform "$home_unsupported" Linux arm64 2>&1)
linux_status=$?
set -e
assert_exit_code 1 "$linux_status"
assert_contains "$linux_output" "Linux support is coming soon."

set +e
arch_output=$(run_with_fake_platform "$home_unsupported" Darwin x86_64 2>&1)
arch_status=$?
set -e
assert_exit_code 1 "$arch_status"
assert_contains "$arch_output" "Unsupported architecture: x86_64"
