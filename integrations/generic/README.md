# Generic lifecycle contract

Choose exactly one source owner.

For a launcher that can attach to an existing checkout, let Stackstead own the
source and runtime:

```sh
contract="$(./integrations/generic/create-stackstead-owned.sh my-task)"
worktree="$(printf '%s' "$contract" | jq -r .worktree)"
id="$(printf '%s' "$contract" | jq -r .stackstead_id)"
cd "$worktree"
stackstead run "$id" -- claude
```

For a manager that must create its own worktree, run
the trusted, absolute installed copy of `adopt-current.sh` once from its
blocking post-create hook.
Run `stop-current.sh` when closing a session but retaining it. From an explicit
pre-remove hook only, set `STACKSTEAD_MANAGER_TEARDOWN=1` and run
`destroy-adopted-current.sh` before the manager deletes its checkout.

Use the trusted, absolute installed copy of
`run-current.sh <agent-or-command> [args...]` as a pane or launcher command when
the manager supports one. It reads the exact pointer and delegates to `stackstead
run`, which sets the stackstead directory and generated environment without
parsing a branch name. Never execute a relative lifecycle hook from a
branch-controlled checkout.

The destroy hook validates pointer/manifest identity, exact worktree path, and
`source_ownership: external`. Stackstead removes the exact runtime, volumes,
state, and generated `.stackstead` files, then proves that the manager-owned
checkout still exists. It refuses to act on Stackstead-owned source.
