#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

max_lines=300
file_count=0
violating_file_count=0
paths_file="$(mktemp)"
trap 'rm -f -- "$paths_file"' EXIT

git ls-files --cached --others --exclude-standard -z -- \
  src tests examples benches >"$paths_file"

while IFS= read -r -d '' path; do
  if [[ ! -f "$path" || "$path" != *.rs ]]; then
    continue
  fi
  file_violates=0
  if grep -Fq 'too_many_lines' "$path"; then
    printf 'error: forbidden function-size bypass identifier in %s: too_many_lines\n' \
      "$path" >&2
    file_violates=1
  fi
  lines="$(LC_ALL=C awk 'END { print NR + 0 }' "$path")"
  file_count=$((file_count + 1))
  if ((lines > max_lines)); then
    printf 'error: %s has %d lines (maximum %d)\n' "$path" "$lines" "$max_lines" >&2
    file_violates=1
  fi
  if ((file_violates > 0)); then
    violating_file_count=$((violating_file_count + 1))
  fi
done <"$paths_file"

if ((violating_file_count > 0)); then
  printf 'error: %d Rust source file(s) violate source-size policy (maximum %d physical lines)\n' \
    "$violating_file_count" "$max_lines" >&2
  exit 1
fi

printf 'Rust source size: %d files, configured maximum %d physical lines\n' \
  "$file_count" "$max_lines"
