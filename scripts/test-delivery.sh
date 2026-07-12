#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/stackstead-delivery-test.XXXXXX")"
tmp="$(CDPATH= cd -- "$tmp" && pwd -P)"
trap 'rm -rf "$tmp"' EXIT

for file in LICENSE SECURITY.md CONTRIBUTING.md docs/quickstart.md docs/agent-setup-v1.md; do
  [[ -s "$repo_root/$file" ]]
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

grep -q 'stackstead launch feature-a -- codex' "$repo_root/docs/quickstart.md"
sh -n "$repo_root/scripts/test-release-install.sh"
grep -q 'scripts/test-release-install.sh' "$repo_root/.github/workflows/ci.yml"
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
printf '%s\n' \
  '{"stackstead_id":"manager-cell-a123","manifest":"/attacker/manifest.json","repo_root":"/attacker/repo"}' \
  >"$manager/.stackstead/stackstead.json"
inspection="$tmp/inspection.json"
printf '{"kind":"StacksteadInspection","version":"1","stackstead":{"stackstead_id":"manager-cell-a123","worktree":"%s","files":{"pointer":"%s"},"repo_root":"%s","source_ownership":"external"},"live":{},"warnings":[]}\n' \
  "$manager" "$manager/.stackstead/stackstead.json" "$tmp" >"$inspection"
destroy_marker="$tmp/destroy-was-called"
up_marker="$tmp/up-was-called"
manager_fake="$tmp/fake-manager-stackstead"
cat >"$manager_fake" <<'EOF'
#!/bin/sh
if [ "$1 $2" = "--json inspect" ]; then cat "$FAKE_INSPECTION"; exit 0; fi
if [ "$1" = up ]; then touch "$FAKE_UP_MARKER"; exit 0; fi
if [ "$1" = destroy ]; then touch "$FAKE_DESTROY_MARKER"; exit 0; fi
exit 2
EOF
chmod +x "$manager_fake"
(cd "$manager" && STACKSTEAD_BIN="$manager_fake" FAKE_INSPECTION="$inspection" \
  FAKE_UP_MARKER="$up_marker" "$repo_root/integrations/hooks/adopt-current.sh")
[[ -e "$up_marker" ]]
rm -f "$up_marker"
cp "$inspection" "$inspection.good"
sed "s|\"worktree\":\"$manager\"|\"worktree\":\"$tmp/wrong-worktree\"|" \
  "$inspection.good" >"$inspection"
if (cd "$manager" && STACKSTEAD_BIN="$manager_fake" FAKE_INSPECTION="$inspection" \
  FAKE_UP_MARKER="$up_marker" "$repo_root/integrations/hooks/adopt-current.sh") >/dev/null 2>&1; then
  printf 'error: adoption hook reused a mismatched pointer\n' >&2
  exit 1
fi
[[ ! -e "$up_marker" ]]
cp "$inspection.good" "$inspection"
(cd "$manager" && STACKSTEAD_MANAGER_TEARDOWN=1 STACKSTEAD_BIN="$manager_fake" \
  FAKE_INSPECTION="$inspection" FAKE_DESTROY_MARKER="$destroy_marker" FAKE_UP_MARKER="$up_marker" \
  "$repo_root/integrations/hooks/destroy-adopted-current.sh")
[[ -e "$destroy_marker" ]]
rm -f "$destroy_marker"
sed "s|\"worktree\":\"$manager\"|\"worktree\":\"$tmp/wrong-worktree\"|" \
  "$inspection" >"$inspection.next"
mv "$inspection.next" "$inspection"
if (cd "$manager" && STACKSTEAD_MANAGER_TEARDOWN=1 STACKSTEAD_BIN="$manager_fake" \
  FAKE_INSPECTION="$inspection" FAKE_DESTROY_MARKER="$destroy_marker" \
  "$repo_root/integrations/hooks/destroy-adopted-current.sh") >/dev/null 2>&1; then
  printf 'error: manager teardown trusted a mismatched manifest\n' >&2
  exit 1
fi
[[ ! -e "$destroy_marker" ]]

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
