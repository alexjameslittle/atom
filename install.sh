#!/usr/bin/env sh
set -eu

REPO=${ATOM_INSTALL_REPO:-alexjameslittle/atom}
API_BASE=${ATOM_INSTALL_API_BASE:-https://api.github.com/repos/$REPO}
RELEASES_API=${ATOM_INSTALL_RELEASES_API:-$API_BASE/releases?per_page=1}
DOWNLOAD_BASE=${ATOM_INSTALL_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download}

log() {
  printf '%s\n' "$*"
}

fail() {
  printf 'atom installer: %s\n' "$*" >&2
  exit 1
}

have_cmd() {
  command -v "$1" >/dev/null 2>&1
}

usage() {
  cat <<EOF
Install Atom from GitHub Releases.

Usage:
  sh install.sh [--version <tag>] [--prefix <dir>]

Options:
  --version <tag>  Install a specific release tag (example: v0.1.0)
  --prefix <dir>   Install under <dir>/bin/atom (default: \$HOME/.atom)
  -h, --help       Show this help message
EOF
}

download_text() {
  url=$1

  if have_cmd curl; then
    curl -fsSL "$url"
    return 0
  fi

  if have_cmd wget; then
    wget -qO - "$url"
    return 0
  fi

  fail "curl or wget is required to download Atom."
}

download_file() {
  url=$1
  dest=$2

  if have_cmd curl; then
    curl -fsSL "$url" -o "$dest"
    return 0
  fi

  if have_cmd wget; then
    wget -qO "$dest" "$url"
    return 0
  fi

  fail "curl or wget is required to download Atom."
}

sha256_file() {
  file=$1

  if have_cmd shasum; then
    shasum -a 256 "$file" | awk '{print $1}'
    return 0
  fi

  if have_cmd sha256sum; then
    sha256sum "$file" | awk '{print $1}'
    return 0
  fi

  fail "shasum or sha256sum is required to verify Atom downloads."
}

resolve_latest_version() {
  json=$(download_text "$RELEASES_API")
  version=$(
    printf '%s' "$json" |
      tr '\n' ' ' |
      awk '
        match($0, /"tag_name"[[:space:]]*:[[:space:]]*"[^"]+"/) {
          value = substr($0, RSTART, RLENGTH)
          sub(/^.*"tag_name"[[:space:]]*:[[:space:]]*"/, "", value)
          sub(/"$/, "", value)
          print value
          exit
        }
      '
  )

  if [ -z "$version" ]; then
    fail "No published Atom releases were found."
  fi

  printf '%s\n' "$version"
}

choose_profile() {
  for name in .zshrc .bashrc .bash_profile; do
    path=$HOME/$name
    if [ -f "$path" ]; then
      printf '%s\n' "$path"
      return 0
    fi
  done

  printf '%s\n' "$HOME/.zshrc"
}

path_contains() {
  case ":${PATH:-}:" in
    *:"$1":*) return 0 ;;
    *) return 1 ;;
  esac
}

requested_version=
prefix=${HOME:-}/.atom

if [ -z "${HOME:-}" ]; then
  fail "HOME must be set before running the installer."
fi

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      shift
      [ "$#" -gt 0 ] || fail "--version requires a value."
      requested_version=$1
      ;;
    --version=*)
      requested_version=${1#*=}
      ;;
    --prefix)
      shift
      [ "$#" -gt 0 ] || fail "--prefix requires a value."
      prefix=$1
      ;;
    --prefix=*)
      prefix=${1#*=}
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "Unknown argument: $1"
      ;;
  esac
  shift
done

case "$prefix" in
  /*) ;;
  *) prefix=$(cd -- "$(pwd -P)" && printf '%s/%s\n' "$(pwd -P)" "$prefix") ;;
esac

uname_s=${ATOM_INSTALL_UNAME_S:-$(uname -s)}
uname_m=${ATOM_INSTALL_UNAME_M:-$(uname -m)}

case "$uname_s" in
  Darwin)
    ;;
  Linux)
    fail "Linux support is coming soon."
    ;;
  *)
    fail "Unsupported operating system: $uname_s. Only macOS is supported right now."
    ;;
esac

case "$uname_m" in
  arm64|aarch64)
    asset_name=atom-darwin-arm64
    ;;
  *)
    fail "Unsupported architecture: $uname_m. Only macOS arm64 is supported right now."
    ;;
esac

if [ -n "$requested_version" ]; then
  case "$requested_version" in
    v*) version=$requested_version ;;
    *) version=v$requested_version ;;
  esac
else
  version=$(resolve_latest_version)
fi

bin_dir=$prefix/bin
target=$bin_dir/atom
profile=$(choose_profile)
export_line="export PATH=\"$bin_dir:\$PATH\""

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/atom-install.XXXXXX") || fail "Failed to create a temp dir."
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

binary_tmp=$tmpdir/$asset_name
checksum_tmp=$tmpdir/$asset_name.sha256

binary_url=$DOWNLOAD_BASE/$version/$asset_name
checksum_url=$DOWNLOAD_BASE/$version/$asset_name.sha256

log "Installing Atom ${version}..."
download_file "$binary_url" "$binary_tmp"
download_file "$checksum_url" "$checksum_tmp"

expected_checksum=$(awk '{print $1; exit}' "$checksum_tmp")
[ -n "$expected_checksum" ] || fail "Release checksum file for ${version} was empty."

actual_checksum=$(sha256_file "$binary_tmp")
if [ "$expected_checksum" != "$actual_checksum" ]; then
  fail "SHA256 checksum mismatch for ${asset_name}."
fi

mkdir -p "$bin_dir"
chmod 755 "$binary_tmp"
mv "$binary_tmp" "$target"
chmod 755 "$target"

profile_updated=0
path_ready=0

if path_contains "$bin_dir"; then
  path_ready=1
elif [ -f "$profile" ] && grep -F "$export_line" "$profile" >/dev/null 2>&1; then
  :
else
  printf '\n%s\n' "$export_line" >> "$profile"
  profile_updated=1
fi

log "Atom ${version} installed to ${target}"
if [ "$profile_updated" -eq 1 ]; then
  log "Added ${bin_dir} to PATH in ${profile}"
fi

if [ "$path_ready" -eq 1 ]; then
  log "Run 'atom --help' to get started."
else
  log "Open a new terminal or run: . \"${profile}\""
  log "Then run 'atom --help' to get started."
fi
