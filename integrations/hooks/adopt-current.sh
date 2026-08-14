#!/usr/bin/env bash
set -Eeuo pipefail

stackstead_bin="${STACKSTEAD_BIN:-stackstead}"

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

command -v jq >/dev/null 2>&1 || die "jq is required"
command -v "$stackstead_bin" >/dev/null 2>&1 || die "Stackstead executable not found: $stackstead_bin"

worktree="$(git rev-parse --show-toplevel 2>/dev/null)" || die "hook must run inside a Git worktree"
worktree="$(CDPATH= cd -- "$worktree" && pwd -P)"
primary="$(git -C "$worktree" worktree list --porcelain | awk '/^worktree / { print substr($0, 10); exit }')"
[ -n "$primary" ] || die "could not locate the primary Git worktree"
primary="$(CDPATH= cd -- "$primary" && pwd -P)"
if [ "$worktree" = "$primary" ]; then
  printf 'Stackstead integration: primary worktree is manager-owned and will not be adopted.\n'
  exit 0
fi
branch="$(git -C "$worktree" symbolic-ref --quiet --short HEAD)" || die "detached worktrees need an explicit integration"
name="${STACKSTEAD_NAME:-$(printf '%s' "$branch" | LC_ALL=C sed 's/[^A-Za-z0-9._-]/-/g')}"
[ -n "$name" ] || die "could not derive a safe Stackstead name; set STACKSTEAD_NAME"

if [ -f "$worktree/.stackstead/stackstead.json" ]; then
  pointer="$worktree/.stackstead/stackstead.json"
  current="$(cd "$worktree" && "$stackstead_bin" --json current)"
  id="$(jq -er \
    --arg worktree "$worktree" \
    --arg pointer "$pointer" \
    --arg primary "$primary" \
    'select(
      .kind == "StacksteadCurrent" and .version == "1" and
      (.stackstead_id | type == "string" and length > 0) and
      .worktree == $worktree and
      .pointer == $pointer and
      .repo_root == $primary and
      .source_ownership == "external"
    ) | .stackstead_id' \
    <<<"$current")" || die "existing contract does not bind this exact external worktree"
  (cd "$primary" && "$stackstead_bin" up "$id")
  exit 0
fi

json="$(mktemp "${TMPDIR:-/tmp}/stackstead-adopt.XXXXXX")"
trap 'rm -f "$json"' EXIT
(cd "$primary" && "$stackstead_bin" --json adopt "$name" --worktree "$worktree") >"$json"
jq -e '.kind == "StacksteadChange" and .version == "1" and .action == "adopted" and .stackstead.source_ownership == "external"' \
  "$json" >/dev/null || die "unsupported Stackstead adopt response"
id="$(jq -er '.stackstead.stackstead_id' "$json")"
(cd "$primary" && "$stackstead_bin" up "$id")
