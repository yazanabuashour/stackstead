#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/stackstead-delivery-test.XXXXXX")"
tmp="$(CDPATH= cd -- "$tmp" && pwd -P)"
trap 'rm -rf "$tmp"' EXIT

for file in LICENSE SECURITY.md CONTRIBUTING.md docs/quickstart.md docs/agent-setup.md; do
  [[ -s "$repo_root/$file" ]]
done

output_sources=("$repo_root/src/output.rs")
if [[ -d "$repo_root/src/output" ]]; then
  while IFS= read -r source; do
    output_sources+=("$source")
  done < <(find "$repo_root/src/output" -type f -name '*.rs' -print | LC_ALL=C sort)
fi
json_kinds=()
while IFS= read -r kind; do
  json_kinds+=("$kind")
done < <(
  cat "${output_sources[@]}" |
    tr '\n' ' ' |
    grep -oE 'kind[[:space:]]*:[[:space:]]*"[^"]+"' |
    sed -E 's/.*"([^"]+)"/\1/' |
    LC_ALL=C sort -u
)
[[ "${#json_kinds[@]}" -gt 0 ]] || {
  printf 'error: no top-level JSON kind literals found in the src/output module\n' >&2
  exit 1
}
for kind in "${json_kinds[@]}"; do
  grep -Eq '^\| `stackstead --json [^|]+` \| `'"$kind"'` \|' \
    "$repo_root/docs/agent-contract.md" || {
    printf 'error: CLI JSON table omits implementation kind: %s\n' "$kind" >&2
    exit 1
  }
done

while IFS= read -r document; do
  [[ -f "$repo_root/$document" ]] || continue
  while IFS= read -r link; do
    case "$link" in http://* | https://* | mailto:* | \#*) continue ;; esac
    target="${link%%[?#]*}"
    [[ -e "$repo_root/$(dirname "$document")/$target" ]] || {
      printf 'error: broken local Markdown link: %s -> %s\n' "$document" "$link" >&2
      exit 1
    }
  done < <(grep -oE '\]\([^)]+' "$repo_root/$document" | sed 's/^](//')
done < <(git -C "$repo_root" ls-files '*.md')

grep -q 'stackstead launch feature-a -- claude' "$repo_root/docs/quickstart.md"
grep -q '<!-- stackstead-policy: 1 -->' "$repo_root/docs/agent-setup.md"
sh -n "$repo_root/scripts/test-release-install.sh"
bash -n "$repo_root/scripts/ci.sh"
bash -n "$repo_root/scripts/test-policy.sh"
for mode in rust docker macos; do
  grep -q "scripts/ci.sh $mode" "$repo_root/.github/workflows/ci.yml"
done
grep -q 'scripts/check-policy.sh' "$repo_root/scripts/ci.sh"
grep -q 'scripts/test-policy.sh' "$repo_root/scripts/ci.sh"
grep -q 'scripts/test-release-install.sh' "$repo_root/scripts/ci.sh"
grep -q 'scripts/test-release-install.sh' "$repo_root/.github/workflows/release.yml"

git -C "$tmp" init -b main >/dev/null
git -C "$tmp" -c user.name='Stackstead Delivery Test' \
  -c user.email='delivery-test@stackstead.invalid' commit --allow-empty -m initial >/dev/null
fake="$tmp/fake-stackstead"
marker="$tmp/stackstead-was-called"
printf '#!/bin/sh\ntouch %q\nexit 99\n' "$marker" >"$fake"
chmod +x "$fake"
output="$(cd "$tmp" && STACKSTEAD_BIN="$fake" "$repo_root/integrations/hooks/adopt-current.sh")"
[[ "$output" == *'primary worktree'* ]]
[[ ! -e "$marker" ]]

manager="$tmp/manager-owned"
git -C "$tmp" worktree add -b manager-feature "$manager" main >/dev/null
mkdir -p "$manager/.stackstead"
: >"$manager/.stackstead/stackstead.json"

plain_calls="$tmp/plain-current-calls"
plain_fake="$tmp/fake-plain-current-stackstead"
cat >"$plain_fake" <<'EOF'
#!/bin/sh
printf '[' >>"$FAKE_CALLS"
for argument in "$@"; do printf '<%s>' "$argument" >>"$FAKE_CALLS"; done
printf ']\n' >>"$FAKE_CALLS"
if [ "$1" = current ]; then printf '%s\n' manager-cell-a123; exit 0; fi
if [ "$1" = run ] || [ "$1" = stop ]; then exit 0; fi
exit 2
EOF
chmod +x "$plain_fake"
no_jq_path="$tmp/no-jq-path"
mkdir "$no_jq_path"
ln -s "$(command -v bash)" "$no_jq_path/bash"
(cd "$manager" && PATH="$no_jq_path" STACKSTEAD_BIN="$plain_fake" FAKE_CALLS="$plain_calls" \
  "$repo_root/integrations/hooks/run-current.sh" agent-command 'argument with spaces')
[[ "$(cat "$plain_calls")" == $'[<current>]\n[<run><manager-cell-a123><--><agent-command><argument with spaces>]' ]]
: >"$plain_calls"
(cd "$manager" && PATH="$no_jq_path" STACKSTEAD_BIN="$plain_fake" FAKE_CALLS="$plain_calls" \
  "$repo_root/integrations/hooks/stop-current.sh")
[[ "$(cat "$plain_calls")" == $'[<current>]\n[<stop><manager-cell-a123>]' ]]

current="$tmp/current.json"
write_current() {
  jq -n \
    --arg kind "$1" \
    --arg version "$2" \
    --arg worktree "$3" \
    --arg pointer "$4" \
    --arg repo_root "$5" \
    --arg source_ownership "$6" \
    '{kind: $kind, version: $version, stackstead_id: "manager-cell-a123",
      source_ownership: $source_ownership, repo_root: $repo_root,
      worktree: $worktree, pointer: $pointer}' >"$current"
}
mutation_marker="$tmp/manager-mutation-call"
manager_fake="$tmp/fake-manager-stackstead"
cat >"$manager_fake" <<'EOF'
#!/bin/sh
if [ "$1 $2" = "--json current" ]; then
  [ "$PWD" = "$EXPECTED_WORKTREE" ] || exit 3
  cat "$FAKE_CURRENT"
  exit 0
fi
if [ "$1" = up ] || [ "$1" = destroy ]; then
  [ "$PWD" = "$EXPECTED_PROJECT_ROOT" ] || exit 4
  printf '%s\n' "$*" >"$FAKE_MUTATION_MARKER"
  exit 0
fi
exit 2
EOF
chmod +x "$manager_fake"

run_current_hook() {
  local hook="$1"
  local teardown=
  case "$hook" in
    destroy-adopted-current.sh) teardown=1 ;;
  esac
  (cd "$manager" && STACKSTEAD_MANAGER_TEARDOWN="$teardown" \
    STACKSTEAD_BIN="$manager_fake" FAKE_CURRENT="$current" \
    EXPECTED_WORKTREE="$manager" EXPECTED_PROJECT_ROOT="$tmp" \
    FAKE_MUTATION_MARKER="$mutation_marker" "$repo_root/integrations/hooks/$hook")
}

pointer="$manager/.stackstead/stackstead.json"
write_current StacksteadCurrent 1 "$manager" "$pointer" "$tmp" external
run_current_hook adopt-current.sh
[[ "$(cat "$mutation_marker")" == 'up manager-cell-a123' ]]
run_current_hook destroy-adopted-current.sh
[[ "$(cat "$mutation_marker")" == 'destroy manager-cell-a123 --yes' ]]

for variant in kind version worktree pointer repo ownership; do
  kind=StacksteadCurrent
  version=1
  dto_worktree="$manager"
  dto_pointer="$pointer"
  dto_repo="$tmp"
  ownership=external
  case "$variant" in
    kind) kind=WrongKind ;;
    version) version=999 ;;
    worktree) dto_worktree="$tmp/wrong-worktree" ;;
    pointer) dto_pointer="$tmp/wrong-pointer" ;;
    repo) dto_repo="$tmp/wrong-repository" ;;
    ownership) ownership=stackstead ;;
  esac
  write_current "$kind" "$version" "$dto_worktree" "$dto_pointer" "$dto_repo" "$ownership"
  for hook in adopt-current.sh destroy-adopted-current.sh; do
    rm -f "$mutation_marker"
    if run_current_hook "$hook" >/dev/null 2>&1; then
      printf 'error: %s accepted wrong current %s\n' "$hook" "$variant" >&2
      exit 1
    fi
    [[ ! -e "$mutation_marker" ]]
  done
done

owned_fake="$tmp/fake-owned-stackstead"
cat >"$owned_fake" <<'EOF'
#!/bin/sh
if [ "$1 $2" = "--json create" ]; then
  printf '%s\n' '{"kind":"StacksteadChange","version":"1","action":"created","stackstead":{"stackstead_id":"retained-a123","branch":"retained","worktree":"/tmp/retained","compose_project":"demo-retained-a123","ports":{},"urls":{},"source_ownership":"stackstead"}}'
  exit 0
fi
if [ "$1" = up ]; then exit 19; fi
exit 2
EOF
chmod +x "$owned_fake"
if STACKSTEAD_BIN="$owned_fake" "$repo_root/integrations/generic/create-stackstead-owned.sh" retained \
  >"$tmp/owned.stdout" 2>"$tmp/owned.stderr"; then
  printf 'error: owned integration hid an up failure\n' >&2
  exit 1
fi
grep -q '"stackstead_id": "retained-a123"' "$tmp/owned.stderr"
grep -q '"retained": true' "$tmp/owned.stderr"

demo="$tmp/restartable-demo"
fake_bin="$tmp/fake-bin"
mkdir -p "$demo/manifests" "$fake_bin"
cp "$repo_root/examples/three-agent-demo/demo.sh" "$demo/demo.sh"
for agent in alpha beta gamma; do
  printf '{"stackstead_id":"%s-id","compose_project":"demo-%s"}\n' "$agent" "$agent" \
    >"$demo/manifests/$agent.json"
  printf '%s\t%s-id\t%s/worktree-%s\t%s/manifests/%s.json\tdemo-%s\n' \
    "$agent" "$agent" "$demo" "$agent" "$demo" "$agent" "$agent" \
    >>"$demo/.demo-stacksteads.tsv"
done
printf '%s\n' '#!/bin/sh' \
  'if [ "$1" = compose ] && [ "$2" = version ]; then exit 0; fi' \
  'case "$1 $2 $3" in "ps -aq "|"volume ls -q"|"network ls -q") exit 0;; esac' \
  'exit 0' >"$fake_bin/docker"
printf '%s\n' '#!/bin/sh' \
  'id="$2"' \
  'if [ "$id" = beta-id ] && [ ! -f "$FAKE_ROOT/beta-failed-once" ]; then touch "$FAKE_ROOT/beta-failed-once"; exit 19; fi' \
  'rm -f "$FAKE_ROOT/manifests/${id%-id}.json"' \
  'exit 0' >"$fake_bin/stackstead"
chmod +x "$demo/demo.sh" "$fake_bin/docker" "$fake_bin/stackstead"
if PATH="$fake_bin:$PATH" FAKE_ROOT="$demo" STACKSTEAD_BIN="$fake_bin/stackstead" \
  "$demo/demo.sh" cleanup >/dev/null 2>&1; then
  printf 'error: restartable cleanup fixture did not stop at beta\n' >&2
  exit 1
fi
[[ "$(wc -l <"$demo/.demo-stacksteads.tsv")" -eq 2 ]]
! grep -q '^alpha' "$demo/.demo-stacksteads.tsv"
PATH="$fake_bin:$PATH" FAKE_ROOT="$demo" STACKSTEAD_BIN="$fake_bin/stackstead" \
  "$demo/demo.sh" cleanup >/dev/null
[[ ! -e "$demo/.demo-stacksteads.tsv" ]]

printf 'Delivery contract tests passed.\n'
