#!/usr/bin/env bash
set -Eeuo pipefail

[ "$#" -eq 1 ] || {
  printf 'usage: %s <stackstead-name>\n' "$0" >&2
  exit 2
}
command -v jq >/dev/null 2>&1 || {
  printf 'error: jq is required\n' >&2
  exit 1
}
stackstead_bin="${STACKSTEAD_BIN:-stackstead}"
json="$(mktemp "${TMPDIR:-/tmp}/stackstead-create.XXXXXX")"
trap 'rm -f "$json"' EXIT
"$stackstead_bin" --json create "$1" >"$json"
jq -e '.kind == "StacksteadChange" and .version == "1" and .action == "created"' \
  "$json" >/dev/null || {
  printf 'error: unsupported Stackstead create response\n' >&2
  exit 1
}
id="$(jq -er '.stackstead.stackstead_id' "$json")"
if ! "$stackstead_bin" up "$id" >&2; then
  jq '.stackstead | {stackstead_id, branch, worktree, compose_project, ports, urls, source_ownership, retained: true}' \
    "$json" >&2
  exit 1
fi
jq '.stackstead | {stackstead_id, branch, worktree, compose_project, ports, urls, source_ownership}' "$json"
