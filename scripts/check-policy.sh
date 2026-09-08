#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
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

require_file_setting() {
  local file="$1"
  local setting="$2"
  if ! grep -Fxq "$setting" "$file"; then
    printf 'error: %s must contain: %s\n' "$file" "$setting" >&2
    failed=1
  fi
}

require_setting package 'autolib = false'
require_setting lints.rust 'let_underscore_drop = "deny"'
require_setting lints.rust 'unfulfilled_lint_expectations = "deny"'
require_setting lints.rust 'unsafe_code = "deny"'
require_setting lints.rust 'unsafe_op_in_unsafe_fn = "deny"'
require_setting lints.clippy 'pedantic = { level = "deny", priority = -1 }'
require_setting lints.clippy 'nursery = { level = "deny", priority = -1 }'
require_setting lints.clippy 'allow_attributes = "deny"'
require_setting lints.clippy 'allow_attributes_without_reason = "deny"'
require_setting lints.clippy 'arithmetic_side_effects = "deny"'
require_setting lints.clippy 'as_conversions = "deny"'
require_setting lints.clippy 'as_pointer_underscore = "deny"'
require_setting lints.clippy 'assertions_on_result_states = "deny"'
require_setting lints.clippy 'cfg_not_test = "deny"'
require_setting lints.clippy 'clone_on_ref_ptr = "deny"'
require_setting lints.clippy 'dbg_macro = "deny"'
require_setting lints.clippy 'default_union_representation = "deny"'
require_setting lints.clippy 'disallowed_types = "deny"'
require_setting lints.clippy 'empty_drop = "deny"'
require_setting lints.clippy 'error_impl_error = "deny"'
require_setting lints.clippy 'exit = "deny"'
require_setting lints.clippy 'expect_used = "deny"'
require_setting lints.clippy 'field_scoped_visibility_modifiers = "deny"'
require_setting lints.clippy 'filetype_is_file = "deny"'
require_setting lints.clippy 'fn_to_numeric_cast_any = "deny"'
require_setting lints.clippy 'host_endian_bytes = "deny"'
require_setting lints.clippy 'ignore_without_reason = "deny"'
require_setting lints.clippy 'indexing_slicing = "deny"'
require_setting lints.clippy 'infinite_loop = "deny"'
require_setting lints.clippy 'iter_over_hash_type = "deny"'
require_setting lints.clippy 'let_underscore_must_use = "deny"'
require_setting lints.clippy 'let_underscore_untyped = "deny"'
require_setting lints.clippy 'lossy_float_literal = "deny"'
require_setting lints.clippy 'map_err_ignore = "deny"'
require_setting lints.clippy 'mem_forget = "deny"'
require_setting lints.clippy 'mixed_read_write_in_expression = "deny"'
require_setting lints.clippy 'multiple_unsafe_ops_per_block = "deny"'
require_setting lints.clippy 'panic = "deny"'
require_setting lints.clippy 'panic_in_result_fn = "deny"'
require_setting lints.clippy 'partial_pub_fields = "deny"'
require_setting lints.clippy 'pointer_format = "deny"'
require_setting lints.clippy 'precedence_bits = "deny"'
require_setting lints.clippy 'rest_pat_in_fully_bound_structs = "deny"'
require_setting lints.clippy 'same_name_method = "deny"'
require_setting lints.clippy 'string_slice = "deny"'
require_setting lints.clippy 'suspicious_xor_used_as_pow = "deny"'
require_setting lints.clippy 'todo = "deny"'
require_setting lints.clippy 'too_many_lines = "deny"'
require_setting lints.clippy 'try_err = "deny"'
require_setting lints.clippy 'unchecked_time_subtraction = "deny"'
require_setting lints.clippy 'undocumented_unsafe_blocks = "deny"'
require_setting lints.clippy 'unnecessary_safety_comment = "deny"'
require_setting lints.clippy 'unnecessary_safety_doc = "deny"'
require_setting lints.clippy 'unimplemented = "deny"'
require_setting lints.clippy 'unreachable = "deny"'
require_setting lints.clippy 'unused_result_ok = "deny"'
require_setting lints.clippy 'unwrap_used = "deny"'
require_setting lints.clippy 'wildcard_dependencies = "deny"'
require_setting lints.clippy 'wildcard_enum_match_arm = "deny"'

require_file_setting clippy.toml 'too-many-lines-threshold = 100'
require_file_setting clippy.toml '  { path = "std::any::Any", reason = "Use domain enums or boundary decoders instead of runtime type recovery." },'
require_file_setting clippy.toml '  { path = "core::any::Any", reason = "Use domain enums or boundary decoders instead of runtime type recovery." },'

metadata="$(cargo metadata --no-deps --format-version 1)"
if [ -e src/lib.rs ] ||
  printf '%s\n' "$metadata" |
    grep -Eq '"kind":\[[^]]*"(lib|rlib|dylib|cdylib|staticlib|proc-macro)"'; then
  printf 'error: Stackstead is binary-only; remove the Rust library target\n' >&2
  failed=1
fi

if grep -REq '^[[:space:]]*pub[[:space:]]+(unsafe[[:space:]]+)?mod[[:space:]]' src --include='*.rs'; then
  printf 'error: Stackstead Rust modules must remain private\n' >&2
  failed=1
fi

exit "$failed"
