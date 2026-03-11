#!/usr/bin/env sh
set -eu

if [ "$(uname -s)" != "Darwin" ]; then
  exit 0
fi

if ! command -v brew >/dev/null 2>&1; then
  exit 0
fi

if ! brew tap | grep -q '^facebook/fb$'; then
  brew tap facebook/fb
fi

if ! command -v idb_companion >/dev/null 2>&1; then
  brew install idb-companion
fi

if command -v idb >/dev/null 2>&1 && idb --help >/dev/null 2>&1; then
  exit 0
fi

if ! command -v python3 >/dev/null 2>&1; then
  brew install python
fi

brew_prefix=$(brew --prefix)
python_bin=$(command -v python3)
python_version=$(
  "$python_bin" - <<'PY'
import sys
print(f"{sys.version_info.major}.{sys.version_info.minor}")
PY
)
site_packages="$brew_prefix/lib/python$python_version/site-packages"

"$python_bin" -m pip install --upgrade --prefix "$brew_prefix" fb-idb

# The pip-generated entrypoint can point at Xcode's python without adding the
# Homebrew prefix site-packages path where fb-idb was installed. Overwrite it
# with a stable wrapper that wires the matching PYTHONPATH before dispatch.
cat >"$brew_prefix/bin/idb" <<EOF
#!/usr/bin/env sh
set -eu
export PYTHONPATH="$site_packages\${PYTHONPATH:+:\$PYTHONPATH}"
exec "$python_bin" -m idb.cli.main "\$@"
EOF
chmod +x "$brew_prefix/bin/idb"

if ! "$brew_prefix/bin/idb" --help >/dev/null 2>&1; then
  echo "idb install failed: expected the fb-idb CLI to be available on PATH after setup" >&2
  exit 69
fi
