#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
stackstead_bin="${STACKSTEAD_BIN:-$repo_root/target/debug/stackstead}"
temporary_parent=
if [ -n "${STACKSTEAD_DOCKER_TEST_DIR:-}" ]; then
  destination="$STACKSTEAD_DOCKER_TEST_DIR"
else
  temporary_parent="$(mktemp -d "${TMPDIR:-/tmp}/stackstead-docker-integration.XXXXXX")"
  destination="$temporary_parent/run"
fi
owner_token="stackstead-docker-test-$$-$(date +%s)"
owner_file="$destination/.stackstead-docker-test-owner"
owned=0

cleanup() {
  status=$?
  trap - EXIT
  [ "$owned" = 1 ] || { [ -z "$temporary_parent" ] || rmdir "$temporary_parent"; exit "$status"; }
  [ -f "$owner_file" ] && [ "$(cat "$owner_file")" = "$owner_token" ] || exit "$status"
  if [ -x "$destination/demo.sh" ] && [ -s "$destination/.demo-stacksteads.tsv" ]; then
    if ! STACKSTEAD_BIN="$stackstead_bin" "$destination/demo.sh" cleanup; then
      printf 'error: retained failed Docker integration at %s for restartable cleanup\n' "$destination" >&2
      exit 1
    fi
  fi
  destination_suffix="$(printf '%s' "$(basename "$destination")" | tr -c 'A-Za-z0-9._-' '_')"
  rm -rf "$destination"
  rm -rf "$(dirname "$destination")/.stackstead-state-$destination_suffix"
  if [ -n "$temporary_parent" ]; then
    rmdir "$temporary_parent"
  fi
  exit "$status"
}
trap cleanup EXIT

command -v docker >/dev/null 2>&1 || {
  printf 'error: Docker is required for the mandatory integration test\n' >&2
  exit 127
}
docker compose version >/dev/null 2>&1 || {
  printf 'error: Docker Compose is required for the mandatory integration test\n' >&2
  exit 127
}
[ -x "$stackstead_bin" ] || {
  printf 'error: Stackstead binary not found: %s\n' "$stackstead_bin" >&2
  exit 1
}

STACKSTEAD_PREPARE_OWNER_TOKEN="$owner_token" \
  "$repo_root/examples/three-agent-demo/demo.sh" prepare "$destination"
owned=1
for phase in create verify crash recover verify orphan cleanup negatives; do
  STACKSTEAD_BIN="$stackstead_bin" "$destination/demo.sh" "$phase"
done
