#!/bin/sh

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/stackstead-installer-test.XXXXXX")
cleanup() { rm -rf "$tmp"; }
trap cleanup 0
trap 'exit 1' 1 2 3 15

fail() {
    printf 'test-install: %s\n' "$*" >&2
    exit 1
}

case "$(uname -s)" in
    Linux) platform=unknown-linux-gnu ;;
    Darwin) platform=apple-darwin ;;
    *) fail "unsupported test operating system" ;;
esac
case "$(uname -m)" in
    x86_64|amd64) architecture=x86_64 ;;
    arm64|aarch64) architecture=aarch64 ;;
    *) fail "unsupported test architecture" ;;
esac
asset="stackstead-$architecture-$platform"

sha() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    else
        shasum -a 256 "$1" | awk '{ print $1 }'
    fi
}

make_release() {
    directory=$1
    mkdir -p "$directory"
    printf '#!/bin/sh\nprintf "stackstead fixture 1.2.3\\n"\n' > "$directory/$asset"
    chmod 755 "$directory/$asset"
    printf '%s  %s\n' "$(sha "$directory/$asset")" "$asset" > "$directory/SHA256SUMS"
}

sh -n "$root/install.sh"
sh -n "$root/packaging/homebrew/render-formula.sh"
"$root/install.sh" --help >/dev/null
[ "$("$root/install.sh" --installer-version)" = 1 ] || fail "unexpected installer version"
if HOME= STACKSTEAD_INSTALL_DIR= "$root/install.sh" >/dev/null 2>&1; then
    fail "installer unexpectedly accepted an empty install directory"
fi
if "$root/install.sh" --version ../escape --release-base file:///tmp >/dev/null 2>&1; then
    fail "unsafe version unexpectedly accepted"
fi

pinned="$tmp/releases/download/v1.2.3"
latest="$tmp/releases/latest/download"
make_release "$pinned"
make_release "$latest"

"$root/install.sh" --release-base "file://$tmp/releases" --version v1.2.3 \
    --install-dir "$tmp/pinned-bin"
[ "$("$tmp/pinned-bin/stackstead")" = "stackstead fixture 1.2.3" ] || fail "pinned install failed"

"$root/install.sh" --release-base="file://$tmp/releases" \
    --install-dir="$tmp/latest-bin"
[ -x "$tmp/latest-bin/stackstead" ] || fail "latest install is not executable"

printf 'tampered\n' >> "$pinned/$asset"
before=$(sha "$tmp/pinned-bin/stackstead")
if "$root/install.sh" --release-base "file://$tmp/releases" --version 1.2.3 \
    --install-dir "$tmp/pinned-bin" >/dev/null 2>&1; then
    fail "tampered release unexpectedly installed"
fi
[ "$(sha "$tmp/pinned-bin/stackstead")" = "$before" ] || fail "failed install replaced existing binary"

if "$root/install.sh" --release-base http://example.invalid/releases \
    --install-dir "$tmp/http-bin" >/dev/null 2>&1; then
    fail "plain HTTP release base unexpectedly accepted"
fi

libc_bin="$tmp/libc-bin"
mkdir "$libc_bin"
printf '%s\n' '#!/bin/sh' 'case "$1" in -s) echo Linux;; -m) echo x86_64;; esac' \
    >"$libc_bin/uname"
printf '%s\n' '#!/bin/sh' 'echo "musl libc"' >"$libc_bin/ldd"
chmod +x "$libc_bin/uname" "$libc_bin/ldd"
if PATH="$libc_bin:$PATH" "$root/install.sh" --release-base "file://$tmp/releases" \
    --install-dir "$tmp/musl-bin" >/dev/null 2>&1; then
    fail "installer unexpectedly accepted musl Linux"
fi
printf '%s\n' '#!/bin/sh' 'echo "ldd (GNU libc)"' >"$libc_bin/ldd"
printf '%s\n' '#!/bin/sh' 'echo "glibc 2.34"' >"$libc_bin/getconf"
chmod +x "$libc_bin/getconf"
if PATH="$libc_bin:$PATH" "$root/install.sh" --release-base "file://$tmp/releases" \
    --install-dir "$tmp/old-glibc-bin" >/dev/null 2>&1; then
    fail "installer unexpectedly accepted an unsupported glibc"
fi

printf '%s  %s\n' "$(sha "$latest/$asset")" "$asset" >> "$latest/SHA256SUMS"
if "$root/install.sh" --release-base "file://$tmp/releases" \
    --install-dir "$tmp/duplicate-bin" >/dev/null 2>&1; then
    fail "duplicate checksum unexpectedly accepted"
fi

formula_checksums="$tmp/formula-SHA256SUMS"
for formula_asset in \
    stackstead-aarch64-apple-darwin \
    stackstead-x86_64-apple-darwin \
    stackstead-aarch64-unknown-linux-gnu \
    stackstead-x86_64-unknown-linux-gnu; do
    printf '%064d  %s\n' 0 "$formula_asset" >> "$formula_checksums"
done
STACKSTEAD_REPOSITORY=yazanabuashour/stackstead \
    "$root/packaging/homebrew/render-formula.sh" \
    v1.2.3 "$formula_checksums" "$tmp/stackstead.rb"
grep -q 'version "1.2.3"' "$tmp/stackstead.rb" || fail "formula version was not rendered"
grep -q 'github.com/yazanabuashour/stackstead/releases/download/v1.2.3' "$tmp/stackstead.rb" || fail "formula URL was not rendered"
if grep -q '@[A-Z_]*@' "$tmp/stackstead.rb"; then
    fail "formula contains an unresolved placeholder"
fi
if command -v ruby >/dev/null 2>&1; then
    ruby -c "$tmp/stackstead.rb" >/dev/null
fi
if "$root/packaging/homebrew/render-formula.sh" v1.2.3 "$formula_checksums" \
    "$tmp/injected.rb" 'https://example.invalid/releases";system("id")#' >/dev/null 2>&1; then
    fail "formula renderer accepted a quote injection in RELEASE_BASE"
fi
if STACKSTEAD_REPOSITORY='yazanabuashour/stackstead";system("id")#' \
    "$root/packaging/homebrew/render-formula.sh" v1.2.3 "$formula_checksums" \
    "$tmp/injected.rb" >/dev/null 2>&1; then
    fail "formula renderer accepted an unsafe repository"
fi
if STACKSTEAD_HOMEPAGE='https://example.invalid/a\b' \
    STACKSTEAD_REPOSITORY=yazanabuashour/stackstead \
    "$root/packaging/homebrew/render-formula.sh" v1.2.3 "$formula_checksums" \
    "$tmp/injected.rb" >/dev/null 2>&1; then
    fail "formula renderer accepted an unsafe homepage"
fi

existing_destination="$tmp/existing-docker-destination"
mkdir "$existing_destination"
printf 'preserve\n' > "$existing_destination/marker"
if STACKSTEAD_DOCKER_TEST_DIR="$existing_destination" \
    "$root/scripts/docker-integration.sh" >/dev/null 2>&1; then
    fail "Docker integration accepted an existing cleanup destination"
fi
[ "$(cat "$existing_destination/marker")" = preserve ] ||
    fail "Docker integration removed an existing destination"

printf 'installer packaging tests passed\n'
