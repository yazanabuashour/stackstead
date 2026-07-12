#!/bin/sh

set -eu

INSTALLER_VERSION="1"
repository=${STACKSTEAD_REPOSITORY:-yazanabuashour/stackstead}
release_base=${STACKSTEAD_RELEASE_BASE:-}
version=${STACKSTEAD_VERSION:-latest}
install_dir=${STACKSTEAD_INSTALL_DIR:-}
[ -n "$install_dir" ] || install_dir=${HOME:+$HOME/.local/bin}

usage() {
    cat <<'EOF'
Install a checksummed Stackstead release binary.

Usage:
  install.sh [options]

Options:
  --repository REPOSITORY
                         GitHub repository; defaults to yazanabuashour/stackstead
  --release-base URL       Release root (for example, https://host/releases)
  --version VERSION        Release to install; defaults to latest
  --install-dir DIRECTORY  Destination directory; defaults to ~/.local/bin
  --help                   Show this help

Environment variables:
  STACKSTEAD_REPOSITORY, STACKSTEAD_RELEASE_BASE, STACKSTEAD_VERSION,
  STACKSTEAD_INSTALL_DIR

Use --repository or --release-base to install from a fork or mirror. A pinned
version may be written as 1.2.3 or v1.2.3.
EOF
}

die() {
    printf 'stackstead installer: %s\n' "$*" >&2
    exit 1
}

need_value() {
    [ "$#" -ge 2 ] && [ -n "$2" ] || die "$1 requires a value"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repository)
            need_value "$@"
            repository=$2
            shift 2
            ;;
        --repository=*) repository=${1#*=}; shift ;;
        --release-base)
            need_value "$@"
            release_base=$2
            shift 2
            ;;
        --release-base=*) release_base=${1#*=}; shift ;;
        --version)
            need_value "$@"
            version=$2
            shift 2
            ;;
        --version=*) version=${1#*=}; shift ;;
        --install-dir)
            need_value "$@"
            install_dir=$2
            shift 2
            ;;
        --install-dir=*) install_dir=${1#*=}; shift ;;
        -h|--help) usage; exit 0 ;;
        --installer-version) printf '%s\n' "$INSTALLER_VERSION"; exit 0 ;;
        --) shift; break ;;
        *) die "unknown option: $1 (try --help)" ;;
    esac
done

[ "$#" -eq 0 ] || die "unexpected argument: $1"
[ -n "$install_dir" ] || die "--install-dir is required when HOME is unset"

case "$version" in
    latest) tag=latest ;;
    v*) tag=$version ;;
    *) tag=v$version ;;
esac
case "$tag" in
    latest|v[0-9A-Za-z]* ) ;;
    *) die "invalid version: $version" ;;
esac
case "$tag" in
    *[!0-9A-Za-z._+-]* ) die "invalid version: $version" ;;
esac

if [ -z "$release_base" ]; then
    case "$repository" in
        */*) ;;
        *) die "pass --repository for a fork or mirror, or pass --release-base URL" ;;
    esac
    case "$repository" in
        *[!0-9A-Za-z._/-]*|*//*|/*|*/|*/*/*) die "invalid GitHub repository: $repository" ;;
    esac
    release_base="https://github.com/$repository/releases"
fi
release_base=${release_base%/}
case "$release_base" in
    https://*|file://*) ;;
    *) die "release base must use https:// (or file:// for local testing)" ;;
esac

command -v curl >/dev/null 2>&1 || die "curl is required"

case "$(uname -s)" in
    Linux) platform=unknown-linux-gnu ;;
    Darwin) platform=apple-darwin ;;
    *) die "unsupported operating system: $(uname -s)" ;;
esac
case "$(uname -m)" in
    x86_64|amd64) architecture=x86_64 ;;
    arm64|aarch64) architecture=aarch64 ;;
    *) die "unsupported architecture: $(uname -m)" ;;
esac

if [ "$platform" = unknown-linux-gnu ]; then
    if ldd --version 2>&1 | grep -qi musl; then
        die "musl Linux is not supported by the glibc release binary; build from source"
    fi
    libc=$(getconf GNU_LIBC_VERSION 2>/dev/null) ||
        die "could not verify glibc; musl and unknown Linux libcs are not supported"
    case "$libc" in
        glibc\ *) glibc_version=${libc#glibc } ;;
        *) die "unsupported Linux libc: $libc" ;;
    esac
    case "$architecture" in
        x86_64) minimum_glibc=2.35 ;;
        aarch64) minimum_glibc=2.39 ;;
    esac
    awk -v have="$glibc_version" -v need="$minimum_glibc" 'BEGIN {
        split(have, h, "."); split(need, n, ".")
        exit !((h[1] + 0 > n[1] + 0) || (h[1] + 0 == n[1] + 0 && h[2] + 0 >= n[2] + 0))
    }' || die "glibc $glibc_version is too old; $architecture releases require glibc $minimum_glibc or newer"
fi

target="$architecture-$platform"
asset="stackstead-$target"
if [ "$tag" = latest ]; then
    download_root="$release_base/latest/download"
else
    download_root="$release_base/download/$tag"
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/stackstead-install.XXXXXX") || die "could not create a temporary directory"
staging_dir=
cleanup() {
    rm -rf "$tmp_dir"
    [ -z "$staging_dir" ] || rm -rf "$staging_dir"
}
trap cleanup 0
trap 'exit 1' 1 2 3 15

download() {
    url=$1
    destination=$2
    case "$url" in
        https://*) protocol='=https' ;;
        file://*) protocol='=file' ;;
        *) die "refusing unsupported download URL: $url" ;;
    esac
    curl --proto "$protocol" --tlsv1.2 --fail --location --silent --show-error \
        --output "$destination" -- "$url"
}

archive="$tmp_dir/$asset"
checksums="$tmp_dir/SHA256SUMS"
printf 'Downloading Stackstead %s for %s...\n' "$tag" "$target" >&2
download "$download_root/$asset" "$archive"
download "$download_root/SHA256SUMS" "$checksums"

expected=$(awk -v asset="$asset" '
    {
        name = $2
        sub(/^\*/, "", name)
        if (name == asset && length($1) == 64 && $1 !~ /[^0-9A-Fa-f]/) {
            count++
            digest = tolower($1)
        }
    }
    END { if (count == 1) print digest }
' "$checksums")
[ -n "$expected" ] || die "SHA256SUMS does not contain exactly one valid checksum for $asset"

if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$archive" | awk '{ print $1 }')
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$archive" | awk '{ print $1 }')
else
    die "sha256sum or shasum is required"
fi
actual=$(printf '%s' "$actual" | tr 'A-F' 'a-f')
[ "$actual" = "$expected" ] || die "checksum verification failed for $asset"

mkdir -p "$install_dir" || die "could not create install directory: $install_dir"
staging_dir=$(mktemp -d "$install_dir/.stackstead.install.XXXXXX") || die "could not stage in install directory: $install_dir"
staged="$staging_dir/stackstead"
cp "$archive" "$staged" || die "could not write to install directory: $install_dir"
chmod 755 "$staged"
mv -f "$staged" "$install_dir/stackstead"
rmdir "$staging_dir"
staging_dir=
printf 'Installed Stackstead to %s/stackstead\n' "$install_dir" >&2
