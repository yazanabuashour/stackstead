#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

rust_checks() {
  cargo fmt --check
  cargo clippy --locked --all-targets --all-features -- -D warnings
  cargo test --locked
  scripts/test-policy.sh
  scripts/test-install.sh
  cargo build --locked --release
  scripts/test-release-install.sh target/release/stackstead
  scripts/test-delivery.sh
}

docker_checks() {
  cargo build --locked
  scripts/docker-integration.sh
}

macos_checks() {
  cargo test --locked
  scripts/test-install.sh
  scripts/test-delivery.sh
}

mode="${1:-all}"
if [ "$#" -gt 1 ]; then
  printf 'usage: %s [all|rust|docker|macos]\n' "$0" >&2
  exit 2
fi

scripts/check-policy.sh
case "$mode" in
all)
  rust_checks
  docker_checks
  ;;
rust)
  rust_checks
  ;;
docker)
  docker_checks
  ;;
macos)
  macos_checks
  ;;
*)
  printf 'error: unknown CI mode: %s\n' "$mode" >&2
  printf 'usage: %s [all|rust|docker|macos]\n' "$0" >&2
  exit 2
  ;;
esac
