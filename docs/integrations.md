# Integrating existing agent and worktree managers

Stackstead should be the runtime substrate, not another dashboard. An editor,
tmux multiplexer, or agent manager can keep ownership of sessions and prompts
while Stackstead supplies the exact Compose project, ports, database, URLs,
context, recovery, and teardown contract.

There are two valid ownership modes:

| Source owner | Entry point | Stackstead destroy behavior |
| --- | --- | --- |
| Stackstead | `stackstead create <name>` | Removes the validated Stackstead worktree after safe runtime teardown. |
| External manager | `stackstead adopt <name> --worktree <absolute-path>` | Records `source_ownership: external`; removes runtime/state and generated contract files but preserves the manager's checkout. |

Never call both a manager's create command and `stackstead create` for the same
branch. When the manager must create source, use `adopt`. When the manager can
attach to an existing checkout, prefer Stackstead ownership and pass the
machine-readable `.stackstead.worktree` path from the `StacksteadChange` response to it.

The checked-in hooks require Bash, `jq`, and Stackstead on `PATH` (or
`STACKSTEAD_BIN`). Install them outside every branch-controlled checkout, then
replace `/absolute/trusted/stackstead-hooks` in the fragments with that absolute
owner-controlled directory:

```sh
install -d "$HOME/.local/libexec/stackstead-hooks"
install -m 0755 integrations/hooks/*.sh "$HOME/.local/libexec/stackstead-hooks/"
```

When upgrading across a documented inspection JSON version change, install the
new hooks before replacing the Stackstead binary. The current hooks accept
inspection versions 1 and 2 so the old binary remains usable during rollout or
rollback.

Do not invoke a relative hook script from a managed worktree: a branch could
replace it before a manager lifecycle event. `adopt-current.sh` is idempotent:
an existing Stackstead pointer causes `up` for that exact identity rather than a
second adoption. It converts Git ref separators to safe Stackstead separators;
set `STACKSTEAD_NAME` when a manager needs an explicit name. The hook detects the
primary Git worktree and exits without adopting it, so a normal switch to
`main` cannot turn the repository checkout into an external Stackstead. Before
reusing an existing pointer, the hook asks the primary checkout for trusted
`inspect --json` output and requires the exact external worktree, pointer, and
ID; a copied branch pointer cannot start another environment.

## Worktrunk

Worktrunk's checked-in `.config/wt.toml` supports blocking `pre-start` and
`pre-remove` hooks. Merge `integrations/worktrunk/wt.fragment.toml` into it.
The blocking create hook ensures the contract exists before the agent starts;
the blocking remove hook fails the manager operation if Stackstead cannot safely
tear down the external runtime. Preview and approve the project hooks through
Worktrunk before using them.

Source: [Worktrunk hook reference](https://worktrunk.dev/hook/).

## workmux

Merge `integrations/workmux/workmux.fragment.yaml` into `.workmux.yaml` after
replacing its trusted absolute hook path.
workmux documents that `post_create` and `pre_remove` run in the managed
worktree and that a failing `pre_remove` aborts removal. The Stackstead hook runs
before tmux panes open, so agents receive `.stackstead/AGENT_CONTEXT.md` and the
generated environment immediately.

Source: [workmux lifecycle hooks](https://github.com/raine/workmux#lifecycle-hooks).

## webmux

Merge `integrations/webmux/webmux.fragment.yaml` into `.webmux.yaml` after
replacing its trusted absolute hook path. webmux
provides `WEBMUX_WORKTREE_PATH` to `postCreate` and `preRemove`; the fragment
changes into that exact path and never derives a runtime identity from a branch
name. A failed cleanup blocks removal rather than orphaning a live database.

Source: [webmux lifecycle and runtime reference](https://webmux.dev/docs/).

## Generic launchers and coding agents

See `integrations/generic/README.md`. A launcher can consume the JSON returned
by `create-stackstead-owned.sh` and invoke `stackstead run <full-id> -- codex` (or
another agent/command). For manager panes already inside the checkout, use the
trusted installed `run-current.sh codex`; it reads the pointer instead of
guessing from a branch name. The agent should read the generated context and use
`stackstead inspect <full-id> --json` before touching ports, logs, databases,
recovery, or teardown.

For externally owned worktrees the lifecycle is:

```text
manager creates source
  -> adopt-current.sh (adopt + up)
  -> agent/session uses generated contract
  -> stop-current.sh (optional close without deletion)
  -> destroy-adopted-current.sh (explicit pre-remove)
  -> manager removes its preserved checkout
```

The teardown script requires `STACKSTEAD_MANAGER_TEARDOWN=1`. It derives the
primary repository independently from Git, reads only the candidate ID from the
branch-writable pointer, and asks that primary checkout for `inspect --json`.
The trusted result must bind the exact current worktree, pointer, repository,
ID, and `source_ownership: external` before the script passes the full ID to
`destroy --yes`. It then verifies that Stackstead did not delete manager-owned source.
Do not put it in background or post-remove hooks: Stackstead must inspect the
checkout and may refuse dirty or tampered state before the manager deletes it.
