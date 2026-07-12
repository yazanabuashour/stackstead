#!/bin/sh

set -eu

usage() {
    cat <<'EOF'
Usage: render-formula.sh VERSION SHA256SUMS OUTPUT [RELEASE_BASE]

Render the Stackstead Homebrew formula from release-asset checksums. Override
STACKSTEAD_REPOSITORY or pass RELEASE_BASE to render for a fork or mirror.
EOF
}

[ "$#" -ge 3 ] && [ "$#" -le 4 ] || { usage >&2; exit 2; }

version=${1#v}
checksums=$2
output=$3
release_base=${4:-${STACKSTEAD_RELEASE_BASE:-}}
repository=${STACKSTEAD_REPOSITORY:-yazanabuashour/stackstead}

safe_url() {
    [ -n "$1" ] && ! printf %s "$1" | LC_ALL=C grep -Eq '[^A-Za-z0-9._~:/?#@!$&()*+,;=%-]'
}

case "$version" in
    ''|*[!0-9A-Za-z._+-]*) printf 'invalid version: %s\n' "$version" >&2; exit 2 ;;
esac
[ -f "$checksums" ] || { printf 'checksum file not found: %s\n' "$checksums" >&2; exit 2; }

if [ -z "$release_base" ]; then
    printf %s "$repository" | LC_ALL=C grep -Eq '^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$' || {
        printf 'set STACKSTEAD_REPOSITORY for a fork or mirror, or pass RELEASE_BASE\n' >&2
        exit 2
    }
    release_base="https://github.com/$repository/releases"
fi
release_base=${release_base%/}
case "$release_base" in
    https://*) ;;
    *) printf 'Homebrew release base must use https://\n' >&2; exit 2 ;;
esac
homepage=${STACKSTEAD_HOMEPAGE:-${release_base%/releases}}
safe_url "$release_base" || { printf 'invalid Homebrew release base\n' >&2; exit 2; }
safe_url "$homepage" || { printf 'invalid Homebrew homepage\n' >&2; exit 2; }
case "$homepage" in
    https://*) ;;
    *) printf 'Homebrew homepage must use https://\n' >&2; exit 2 ;;
esac

checksum() {
    awk -v asset="$1" '
        {
            name = $2
            sub(/^\*/, "", name)
            if (name == asset && length($1) == 64 && $1 !~ /[^0-9A-Fa-f]/) {
                count++
                digest = tolower($1)
            }
        }
        END { if (count == 1) print digest }
    ' "$checksums"
}

sha_darwin_arm64=$(checksum stackstead-aarch64-apple-darwin)
sha_darwin_x86_64=$(checksum stackstead-x86_64-apple-darwin)
sha_linux_arm64=$(checksum stackstead-aarch64-unknown-linux-gnu)
sha_linux_x86_64=$(checksum stackstead-x86_64-unknown-linux-gnu)
for value in "$sha_darwin_arm64" "$sha_darwin_x86_64" "$sha_linux_arm64" "$sha_linux_x86_64"; do
    [ -n "$value" ] || { printf 'missing or duplicate platform checksum\n' >&2; exit 2; }
done

escape_sed() { printf '%s' "$1" | sed 's/[\\&|]/\\&/g'; }
template_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
mkdir -p "$(dirname -- "$output")"
sed \
    -e "s|@HOMEPAGE@|$(escape_sed "$homepage")|g" \
    -e "s|@RELEASE_BASE@|$(escape_sed "$release_base")|g" \
    -e "s|@VERSION@|$(escape_sed "$version")|g" \
    -e "s|@SHA_DARWIN_ARM64@|$sha_darwin_arm64|g" \
    -e "s|@SHA_DARWIN_X86_64@|$sha_darwin_x86_64|g" \
    -e "s|@SHA_LINUX_ARM64@|$sha_linux_arm64|g" \
    -e "s|@SHA_LINUX_X86_64@|$sha_linux_x86_64|g" \
    "$template_dir/stackstead.rb.template" > "$output"
