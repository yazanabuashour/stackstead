#!/usr/bin/env bash
set -Eeuo pipefail

[ "$#" -gt 0 ] || {
  printf 'usage: %s <agent-or-command> [args...]\n' "$0" >&2
  exit 2
}
stackstead_bin="${STACKSTEAD_BIN:-stackstead}"
id="$("$stackstead_bin" current)"
exec "$stackstead_bin" run "$id" -- "$@"
