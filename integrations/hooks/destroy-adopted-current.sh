#!/usr/bin/env bash
set -Eeuo pipefail

stackstead_bin="${STACKSTEAD_BIN:-stackstead}"

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

[ "${STACKSTEAD_MANAGER_TEARDOWN:-}" = 1 ] ||
  die "set STACKSTEAD_MANAGER_TEARDOWN=1 only from an explicit manager remove hook"
command -v jq >/dev/null 2>&1 || die "jq is required"

worktree="$(git rev-parse --show-toplevel 2>/dev/null)" || die "hook must run inside the worktree being removed"
worktree="$(CDPATH= cd -- "$worktree" && pwd -P)"
pointer="$worktree/.stackstead/stackstead.json"
[ -f "$pointer" ] || die "no Stackstead pointer in the worktree being removed"
id="$(jq -er '.stackstead_id' "$pointer")"
repo_root="$(git worktree list --porcelain | sed -n 's/^worktree //p' | head -n 1)"
[ -n "$repo_root" ] || die "cannot discover the primary repository worktree"
repo_root="$(CDPATH= cd -- "$repo_root" && pwd -P)"

inspection="$(mktemp)"
trap 'rm -f -- "$inspection"' EXIT HUP INT TERM
(cd "$repo_root" && "$stackstead_bin" --json inspect "$id") >"$inspection"
jq -e \
  --arg id "$id" \
  --arg worktree "$worktree" \
  --arg pointer "$pointer" \
  --arg repo_root "$repo_root" \
  '.kind == "StacksteadInspection" and .version == "1" and
   (.stackstead |
    .stackstead_id == $id and
    .worktree == $worktree and
    .files.pointer == $pointer and
    .repo_root == $repo_root and
    .source_ownership == "external")' \
  "$inspection" >/dev/null || die "trusted manifest does not bind this exact external worktree and pointer"

(cd "$repo_root" && "$stackstead_bin" destroy "$id" --yes)
[ -d "$worktree" ] || die "external worktree disappeared; Stackstead must preserve manager-owned source"
