#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/stackstead-policy-test.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

fail() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

policy_fixture="$tmp/policy"
reset_policy_fixture() {
  rm -rf "$policy_fixture"
  mkdir -p "$policy_fixture/scripts" "$policy_fixture/src"
  cp "$repo_root/scripts/check-policy.sh" "$policy_fixture/scripts/"
  printf 'fn main() {}\n' >"$policy_fixture/src/main.rs"
  cat >"$policy_fixture/Cargo.toml" <<'EOF'
[package]
name = "policy-fixture"
version = "0.0.0"
edition = "2024"
autolib = false

[lints.rust]
unfulfilled_lint_expectations = "deny"
unsafe_code = "deny"
unsafe_op_in_unsafe_fn = "deny"

[lints.clippy]
allow_attributes = "deny"
allow_attributes_without_reason = "deny"
expect_used = "deny"
multiple_unsafe_ops_per_block = "deny"
panic = "deny"
todo = "deny"
undocumented_unsafe_blocks = "deny"
unnecessary_safety_comment = "deny"
unimplemented = "deny"
unreachable = "deny"
unwrap_used = "deny"

[[bin]]
name = "policy-fixture"
path = "src/main.rs"
EOF
}

expect_policy_failure() {
  local description="$1"
  if "$policy_fixture/scripts/check-policy.sh" >/dev/null 2>&1; then
    fail "policy accepted $description"
  fi
}

validate_policy_manifest() {
  cargo metadata --manifest-path "$policy_fixture/Cargo.toml" --no-deps --format-version 1 \
    >/dev/null || fail "invalid policy test manifest"
}

reset_policy_fixture
"$policy_fixture/scripts/check-policy.sh"

while IFS='|' read -r section setting; do
  reset_policy_fixture
  awk -v drop="$setting" '$0 != drop' "$policy_fixture/Cargo.toml" \
    >"$policy_fixture/Cargo.toml.next"
  mv "$policy_fixture/Cargo.toml.next" "$policy_fixture/Cargo.toml"
  validate_policy_manifest
  expect_policy_failure "missing [$section] $setting"

  reset_policy_fixture
  awk -v drop="$setting" '$0 != drop' "$policy_fixture/Cargo.toml" \
    >"$policy_fixture/Cargo.toml.next"
  {
    printf '\n[package.metadata.misplaced]\n'
    printf '%s\n' "$setting"
  } >>"$policy_fixture/Cargo.toml.next"
  mv "$policy_fixture/Cargo.toml.next" "$policy_fixture/Cargo.toml"
  validate_policy_manifest
  expect_policy_failure "misplaced [$section] $setting"
done <<'EOF'
package|autolib = false
lints.rust|unfulfilled_lint_expectations = "deny"
lints.rust|unsafe_code = "deny"
lints.rust|unsafe_op_in_unsafe_fn = "deny"
lints.clippy|allow_attributes = "deny"
lints.clippy|allow_attributes_without_reason = "deny"
lints.clippy|expect_used = "deny"
lints.clippy|multiple_unsafe_ops_per_block = "deny"
lints.clippy|panic = "deny"
lints.clippy|todo = "deny"
lints.clippy|undocumented_unsafe_blocks = "deny"
lints.clippy|unnecessary_safety_comment = "deny"
lints.clippy|unimplemented = "deny"
lints.clippy|unreachable = "deny"
lints.clippy|unwrap_used = "deny"
EOF

reset_policy_fixture
printf 'pub fn library() {}\n' >"$policy_fixture/src/lib.rs"
expect_policy_failure src/lib.rs

for header in '[lib] # explicit target' '[ lib ]'; do
  reset_policy_fixture
  printf 'pub fn library() {}\n' >"$policy_fixture/src/custom.rs"
  {
    printf '\n%s\n' "$header"
    printf 'path = "src/custom.rs"\n'
  } >>"$policy_fixture/Cargo.toml"
  validate_policy_manifest
  expect_policy_failure "$header"
done

reset_policy_fixture
printf '#[unsafe(no_mangle)] pub extern "C" fn fixture() {}\n' \
  >"$policy_fixture/src/custom.rs"
cat >>"$policy_fixture/Cargo.toml" <<'EOF'

[lib]
path = "src/custom.rs"
crate-type = ["cdylib"]
EOF
validate_policy_manifest
expect_policy_failure 'an explicit cdylib target'

ci_fixture="$tmp/ci"
mkdir -p "$ci_fixture/scripts" "$ci_fixture/fake-bin"
cp "$repo_root/scripts/ci.sh" "$ci_fixture/scripts/"
cat >"$ci_fixture/scripts/check-policy.sh" <<'EOF'
#!/usr/bin/env bash
printf 'check-policy.sh\n' >>"$CI_LOG"
EOF
cat >"$ci_fixture/fake-bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf 'cargo %s\n' "$*" >>"$CI_LOG"
if [ "${CI_FAIL:-}" = "cargo $*" ]; then
  exit 19
fi
EOF
for script in test-policy.sh test-install.sh test-release-install.sh test-delivery.sh docker-integration.sh; do
  cat >"$ci_fixture/scripts/$script" <<'EOF'
#!/usr/bin/env bash
printf '%s' "${0##*/}" >>"$CI_LOG"
if [ "$#" -gt 0 ]; then
  printf ' %s' "$@" >>"$CI_LOG"
fi
printf '\n' >>"$CI_LOG"
if [ "${CI_FAIL:-}" = "${0##*/} $*" ]; then
  exit 19
fi
EOF
done
chmod +x "$ci_fixture/scripts/"*.sh "$ci_fixture/fake-bin/cargo"

assert_log() {
  local expected="$1"
  diff -u "$expected" "$CI_LOG" || fail "unexpected CI command sequence"
}

export CI_LOG="$tmp/ci.log"
export PATH="$ci_fixture/fake-bin:$PATH"

: >"$CI_LOG"
"$ci_fixture/scripts/ci.sh" rust
cat >"$tmp/rust.expected" <<'EOF'
check-policy.sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
test-policy.sh
test-install.sh
cargo build --locked --release
test-release-install.sh target/release/stackstead
test-delivery.sh
EOF
assert_log "$tmp/rust.expected"

: >"$CI_LOG"
"$ci_fixture/scripts/ci.sh" docker
cat >"$tmp/docker.expected" <<'EOF'
check-policy.sh
cargo build --locked
docker-integration.sh
EOF
assert_log "$tmp/docker.expected"

: >"$CI_LOG"
"$ci_fixture/scripts/ci.sh" macos
cat >"$tmp/macos.expected" <<'EOF'
check-policy.sh
cargo test --locked
test-install.sh
test-delivery.sh
EOF
assert_log "$tmp/macos.expected"

: >"$CI_LOG"
"$ci_fixture/scripts/ci.sh"
cat >"$tmp/all.expected" <<'EOF'
check-policy.sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
test-policy.sh
test-install.sh
cargo build --locked --release
test-release-install.sh target/release/stackstead
test-delivery.sh
cargo build --locked
docker-integration.sh
EOF
assert_log "$tmp/all.expected"

: >"$CI_LOG"
if "$ci_fixture/scripts/ci.sh" unknown >/dev/null 2>&1; then
  fail 'CI accepted an unknown mode'
fi
printf 'check-policy.sh\n' >"$tmp/unknown.expected"
assert_log "$tmp/unknown.expected"

: >"$CI_LOG"
if "$ci_fixture/scripts/ci.sh" rust extra >/dev/null 2>&1; then
  fail 'CI accepted an extra argument'
fi
test ! -s "$CI_LOG" || fail 'CI ran commands before rejecting an extra argument'

: >"$CI_LOG"
if CI_FAIL='cargo test --locked' "$ci_fixture/scripts/ci.sh" rust >/dev/null 2>&1; then
  fail 'CI ignored a command failure'
fi
cat >"$tmp/failure.expected" <<'EOF'
check-policy.sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
EOF
assert_log "$tmp/failure.expected"

printf 'Policy and CI orchestration tests passed.\n'
