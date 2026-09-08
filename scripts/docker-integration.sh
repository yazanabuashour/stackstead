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
destination_parent="$(CDPATH= cd -- "$(dirname -- "$destination")" && pwd -P)"
destination="$destination_parent/$(basename -- "$destination")"
destination_suffix="$(printf '%s' "$(basename "$destination")" | tr -c 'A-Za-z0-9._-' '_')"
state_root="$(dirname "$destination")/.stackstead-state-$destination_suffix"
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
      printf 'Retry: ' >&2
      printf '%q ' "${docker_connection[@]}" "XDG_STATE_HOME=$XDG_STATE_HOME" \
        "STACKSTEAD_BIN=$stackstead_bin" "$destination/demo.sh" cleanup >&2
      printf '\n' >&2
      exit 1
    fi
  fi
  rm -rf "$destination" "$state_root"
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

stackstead_bin="$(CDPATH= cd -- "$(dirname -- "$stackstead_bin")" && pwd -P)/$(basename -- "$stackstead_bin")"
# Retrying must not inherit a different daemon or credential/configuration location.
docker_connection=(env -u HOME -u DOCKER_HOST -u DOCKER_CONTEXT -u DOCKER_CONFIG
  -u DOCKER_TLS_VERIFY -u DOCKER_CERT_PATH)
for key in HOME DOCKER_HOST DOCKER_CONTEXT DOCKER_CONFIG DOCKER_TLS_VERIFY DOCKER_CERT_PATH; do
  if [ "${!key+x}" ]; then
    docker_connection+=("$key=${!key}")
  fi
done
if [ -z "${DOCKER_HOST:-}" ] && [ -z "${DOCKER_CONTEXT:-}" ]; then
  docker_connection+=("DOCKER_CONTEXT=$(docker context show)")
fi
STACKSTEAD_PREPARE_OWNER_TOKEN="$owner_token" \
  "$repo_root/examples/three-agent-demo/demo.sh" prepare "$destination"
owned=1
# Keep the fixture registry with retained project state, without changing Docker's HOME.
export XDG_STATE_HOME="$state_root/user-state"
for phase in create verify crash recover verify orphan cleanup negatives; do
  STACKSTEAD_BIN="$stackstead_bin" "$destination/demo.sh" "$phase"
done
