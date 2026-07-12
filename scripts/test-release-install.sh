#!/bin/sh

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:-$root/target/release/stackstead}
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -n 1)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/stackstead-release-install.XXXXXX")
trap 'rm -rf "$tmp"' 0
trap 'exit 1' 1 2 3 15

[ -n "$version" ] || { printf 'error: Cargo package version is missing\n' >&2; exit 1; }
[ -x "$binary" ] || { printf 'error: release binary is not executable: %s\n' "$binary" >&2; exit 1; }
[ "$("$binary" --version)" = "stackstead $version" ] || {
  printf 'error: release binary version does not match Cargo.toml\n' >&2
  exit 1
}

case "$(uname -s)" in
  Linux) platform=unknown-linux-gnu ;;
  Darwin) platform=apple-darwin ;;
  *) printf 'error: unsupported release test operating system\n' >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) architecture=x86_64 ;;
  arm64|aarch64) architecture=aarch64 ;;
  *) printf 'error: unsupported release test architecture\n' >&2; exit 1 ;;
esac
asset="stackstead-$architecture-$platform"
release="$tmp/releases/download/v$version"
mkdir -p "$release" "$tmp/home"
cp "$binary" "$release/$asset"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$release" && sha256sum "$asset" >SHA256SUMS)
else
  (cd "$release" && shasum -a 256 "$asset" >SHA256SUMS)
fi

HOME="$tmp/home" "$root/install.sh" \
  --release-base "file://$tmp/releases" \
  --version "$version" \
  --install-dir "$tmp/home/.local/bin"

installed=$(env -i HOME="$tmp/home" PATH="$tmp/home/.local/bin:/usr/bin:/bin" \
  stackstead --version)
[ "$installed" = "stackstead $version" ] || {
  printf 'error: clean environment could not execute the installed release\n' >&2
  exit 1
}

printf 'real release binary clean-install test passed\n'
