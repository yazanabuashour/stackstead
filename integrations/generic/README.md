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

The [creation wrapper](create-stackstead-owned.sh) validates `StacksteadChange`
version 1 and action `created`, reads `.stackstead.stackstead_id`, and starts that
exact environment. It emits a flattened projection of `.stackstead`, so the
example above reads `.stackstead_id` and `.worktree` at the top level. Raw CLI
`create` and `adopt` responses keep those fields under `.stackstead`.

For a manager that must create its own worktree, run
the trusted, absolute installed copy of `adopt-current.sh` once from its
blocking post-create hook.
Run `stop-current.sh` when closing a session but retaining it. From an explicit
pre-remove hook only, set `STACKSTEAD_MANAGER_TEARDOWN=1` and run
`destroy-adopted-current.sh` before the manager deletes its checkout.

Use the trusted, absolute installed copy of
`run-current.sh <agent-or-command> [args...]` as a pane or launcher command when
the manager supports one. It delegates through the validated ID from
`stackstead current`, then `stackstead run` sets the worktree and generated
environment without parsing a branch name. A launcher can also call these
commands directly:

```sh
id="$(stackstead current)"; stackstead run "$id" -- true
```

Never execute a relative lifecycle hook from a branch-controlled checkout.
`run-current.sh` and `stop-current.sh` require Bash and Stackstead; they do not
require `jq`. Adoption and destruction also require `jq` to validate
`StacksteadCurrent` version 1.

The destroy hook validates the current identity, exact repository, worktree and
pointer paths, and `source_ownership: external`, then delegates from the primary
checkout so project configuration anchors state lookup. Stackstead removes the
exact runtime, volumes, state, and generated `.stackstead` files, then proves
that the manager-owned checkout still exists. It refuses to act on
Stackstead-owned source.
