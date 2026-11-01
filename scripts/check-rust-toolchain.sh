#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# MSRV resolution requires Python 3.11+ (stdlib tomllib).
if [[ "${1:-}" == --print-msrv ]]; then
  python3 - <<'PY'
import re
import tomllib

with open("Cargo.toml", "rb") as manifest:
    data = tomllib.load(manifest)
package = data.get("package", data.get("workspace", {}).get("package", {}))
version = package.get("rust-version")
if version == {"workspace": True}:
    version = data.get("workspace", {}).get("package", {}).get("rust-version")
if not isinstance(version, str) or not re.fullmatch(r"1\.(0|[1-9][0-9]*)(\.(0|[1-9][0-9]*))?", version):
    raise SystemExit("Cargo.toml must declare a release rust-version in package or workspace.package")
print(version if version.count(".") == 2 else version + ".0")
PY
  exit
fi

pinned_version="$(sed -n 's/^channel = "\([^"]*\)"$/\1/p' rust-toolchain.toml)"
readonly pinned_version
if [[ ! "$pinned_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  printf 'Rust toolchain must use an exact release: %s\n' "$pinned_version" >&2
  exit 1
fi
case "${1:-}" in
  --print)
    printf '%s\n' "$pinned_version"
    ;;
  '')
    actual_version="$(rustc --version | awk '{ print $2 }')"
    if [[ "$actual_version" != "$pinned_version" ]]; then
      printf 'rustc %s does not match pinned Rust %s\n' \
        "$actual_version" "$pinned_version" >&2
      printf 'Select the toolchain from rust-toolchain.toml before running CI\n' >&2
      exit 1
    fi
    ;;
  *)
    printf 'usage: %s [--print|--print-msrv]\n' "$0" >&2
    exit 2
    ;;
esac
