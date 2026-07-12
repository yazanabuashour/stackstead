#!/usr/bin/env bash
set -Eeuo pipefail

[ "$#" -gt 0 ] || {
  printf 'usage: %s <agent-or-command> [args...]\n' "$0" >&2
  exit 2
}
command -v jq >/dev/null 2>&1 || {
  printf 'error: jq is required\n' >&2
  exit 1
}
stackstead_bin="${STACKSTEAD_BIN:-stackstead}"
pointer="$(git rev-parse --show-toplevel 2>/dev/null)/.stackstead/stackstead.json"
[ -f "$pointer" ] || {
  printf 'error: no Stackstead pointer in this worktree\n' >&2
  exit 1
}
id="$(jq -er '.stackstead_id' "$pointer")"
exec "$stackstead_bin" run "$id" -- "$@"
