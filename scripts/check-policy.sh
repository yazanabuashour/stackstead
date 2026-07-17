#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

failed=0

require_setting() {
  local section="$1"
  local setting="$2"
  if ! awk -v header="[$section]" -v setting="$setting" '
    $0 == header { inside = 1; next }
    inside && /^\[/ { inside = 0 }
    inside && $0 == setting { found = 1 }
    END { exit !found }
  ' Cargo.toml; then
    printf 'error: Cargo.toml [%s] must contain: %s\n' "$section" "$setting" >&2
    failed=1
  fi
}

require_setting package 'autolib = false'
require_setting lints.rust 'unfulfilled_lint_expectations = "deny"'
require_setting lints.rust 'unsafe_code = "deny"'
require_setting lints.rust 'unsafe_op_in_unsafe_fn = "deny"'
require_setting lints.clippy 'allow_attributes = "deny"'
require_setting lints.clippy 'allow_attributes_without_reason = "deny"'
require_setting lints.clippy 'expect_used = "deny"'
require_setting lints.clippy 'multiple_unsafe_ops_per_block = "deny"'
require_setting lints.clippy 'panic = "deny"'
require_setting lints.clippy 'todo = "deny"'
require_setting lints.clippy 'undocumented_unsafe_blocks = "deny"'
require_setting lints.clippy 'unnecessary_safety_comment = "deny"'
require_setting lints.clippy 'unimplemented = "deny"'
require_setting lints.clippy 'unreachable = "deny"'
require_setting lints.clippy 'unwrap_used = "deny"'

metadata="$(cargo metadata --no-deps --format-version 1)"
if [ -e src/lib.rs ] ||
  printf '%s\n' "$metadata" |
    grep -Eq '"kind":\[[^]]*"(lib|rlib|dylib|cdylib|staticlib|proc-macro)"'; then
  printf 'error: Stackstead is binary-only; remove the Rust library target\n' >&2
  failed=1
fi

exit "$failed"
