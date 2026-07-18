# Upgrade from 0.1.3 to 0.1.4

Version 0.1.4 changes inspection JSON from version 1 to version 3 and replaces
event-log teardown recovery with an identity-bound phase journal. It does not
provide a mixed-version bridge.

## Before replacing 0.1.3

1. Finish active agent commands and pause worktree-manager lifecycle automation.
2. Run `stackstead ps`, then inspect every retained environment by its full ID.
3. Complete any started or failed destroy with the same 0.1.3 binary. Retain
   that pinned binary until every retained environment and manager integration
   has been verified after the upgrade.
4. Install 0.1.4 and every trusted manager-hook copy as one maintenance
   operation from a reviewed `v0.1.4` checkout.

Verify both the binary and every installed hook before resuming automation:

```sh
stackstead --version
stackstead doctor
stackstead --json inspect <full-id>

hook_dir="$HOME/.local/libexec/stackstead-hooks"
for source in integrations/hooks/*.sh; do
  cmp -s "$source" "$hook_dir/${source##*/}" || exit 1
done
```

The inspection must report kind `StacksteadInspection` and version `3`. Adapt
`hook_dir` only when the trusted hooks were intentionally installed elsewhere.
A rollback must restore the matching binary and hook pair. Do not use 0.1.3
after 0.1.4 has begun a destroy.

## Recover an already failed 0.1.3 destroy

Use this only when 0.1.3 already removed the runtime and unregistered a
Stackstead-owned worktree, but could not remove container-created root-owned
files. Keep the pinned 0.1.3 binary installed for the eventual destroy retry.

The old inspector depends on generated pointer state that partial cleanup may
already have removed. From the primary repository checkout, stage the
checksummed 0.1.4 binary in a temporary directory and use it only for read-only
path and identity validation:

```sh
target_id=<full-id>
candidate_dir=$(mktemp -d)
trap 'rm -rf "$candidate_dir"' EXIT
curl -fsSL https://github.com/yazanabuashour/stackstead/releases/download/v0.1.4/install.sh \
  | sh -s -- --version 0.1.4 --install-dir "$candidate_dir"
candidate="$candidate_dir/stackstead"
test "$("$candidate" --version)" = "stackstead 0.1.4"
inspection="$candidate_dir/inspection.json"
"$candidate" --json inspect "$target_id" >"$inspection"

jq -e --arg id "$target_id" '
  .kind == "StacksteadInspection" and .version == "3" and
  .stackstead.stackstead_id == $id and
  .stackstead.source_ownership == "stackstead" and
  (.stackstead.worktree | type == "string" and startswith("/"))
' "$inspection" >/dev/null

worktree=$(jq -r '.stackstead.worktree' "$inspection")
test -d "$worktree" && test ! -L "$worktree"
canonical_worktree=$(CDPATH= cd -- "$worktree" && pwd -P)
test "$canonical_worktree" = "$worktree"
printf 'Ownership-repair target: %s\n' "$worktree"
```

The 0.1.4 inspector fails unless the manifest paths identify a direct Stackstead
child of the independently discovered project state root and its existing
ancestors resolve inside that root. Stop if any validation fails. Review the
printed path, then authorize ownership repair only for that exact
Stackstead-owned worktree and retry destroy with the installed 0.1.3 binary:

```sh
sudo chown -R "$(id -u):$(id -g)" "$worktree"
stackstead destroy "$target_id" --yes
```

Confirm that the full ID no longer appears in `stackstead --json ps` before
installing 0.1.4. Do not apply this procedure to an externally owned checkout,
another path, or a target that failed inspection binding.
