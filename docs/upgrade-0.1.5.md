# Upgrade from 0.1.4 to 0.1.5

Version 0.1.5 adds the `StacksteadCurrent` version 1 command contract. The
checked-in manager hooks use it instead of reading pointer files or consuming
inspection version 3. It does not provide a mixed-version bridge.

1. Finish active agent commands and pause worktree-manager lifecycle automation.
2. Complete any started or failed destroy with the 0.1.4 binary.
3. Install 0.1.5 and every trusted manager-hook copy as one maintenance
   operation from a reviewed `v0.1.5` checkout.
4. From a retained generated worktree, verify the new command contract:

   ```sh
   test "$(stackstead --version)" = "stackstead 0.1.5"
   current="$(stackstead --json current)"
   jq -e '
     .kind == "StacksteadCurrent" and .version == "1" and
     (.stackstead_id | type == "string" and length > 0) and
     (.repo_root | type == "string" and startswith("/")) and
     (.worktree | type == "string" and startswith("/")) and
     (.pointer | type == "string" and startswith("/"))
   ' <<<"$current" >/dev/null
   ```

5. Compare every installed hook with the reviewed release checkout before
   resuming automation:

   ```sh
   hook_dir="$HOME/.local/libexec/stackstead-hooks"
   for source in integrations/hooks/*.sh; do
     cmp -s "$source" "$hook_dir/${source##*/}" || exit 1
   done
   ```

Adapt `hook_dir` only when the trusted hooks were intentionally installed
elsewhere. A binary without `current` fails the new hooks instead of falling
back to pointer parsing. A rollback must restore the matching binary and hook
pair; do not resume a teardown with an older binary after a newer one started
it.
