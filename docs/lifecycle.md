# Lifecycle and cleanup

A stackstead moves through explicit source, dependency, runtime, database, and health states. The durable manifest is updated after lifecycle changes, and an append-only event log records useful operation boundaries without storing secrets.

## Discover a project

From the canonical repository, Stackstead climbs parent directories until it finds `stackstead.yaml`. From inside a generated worktree, it first reads `.stackstead/stackstead.json`, which points back to the durable manifest and canonical repository. If neither route succeeds, the command fails instead of guessing.

## Initialize

```sh
stackstead init
```

Initialization discovers a conventional Compose file and creates a project-specific `stackstead.yaml` containing its published services, ports, likely URLs, Postgres contract, generated environment, and HTTP health checks. `stackstead compose plan` is read-only; `stackstead compose apply --yes` can rewrite only unambiguous fixed host ports. Review and commit both files to `source.base` before creating a stackstead.

## Create

```sh
stackstead create feature-a
```

Creation resolves `source.base` to one immutable commit, uses Git's own filtered comparison to prove `stackstead.yaml` and every configured Compose file match it, and creates the branch from that exact commit. Reusing an existing branch is allowed only when it contains the pinned base commit. This prevents a moving branch, stale branch, or CRLF/clean filter from changing the contract between validation and checkout. It then sanitizes the name, allocates a deterministic free port slot under a host-wide per-user lease lock, creates collision-resistant runtime identities, and persists a recoverable pending manifest before reserving ports or publishing source. It holds the environment lock through `post_create` and final readiness. A failed pre-publication create releases its exact lease before deleting recovery state, while a process interruption leaves a manager-resolvable pending manifest that `destroy` can finalize. The port lease remains reserved while a stackstead is stopped and is released only after successful destruction; lifecycle commands fail closed if the lease no longer matches the manifest. It fails with a commit-or-merge instruction instead of publishing a checkout that cannot inherit the reviewed runtime contract.

```text
<state-root>/<project>/<stackstead-id>/
  source/
    .stackstead/
      .env
      AGENT_CONTEXT.md
      stackstead.json
  state/
    manifest.json
    lock
    run.lock
    events.jsonl
    logs/
```

The generated source `.stackstead/` directory is added to that worktree's Git exclude file, not to the repository's tracked `.gitignore`.

Names may be resolved by exact stackstead ID or by a unique slug. If multiple stacksteads share a slug, use the full ID shown by `stackstead ps`.

## Adopt an externally managed checkout

```sh
stackstead adopt feature-a --worktree /absolute/path/to/registered-worktree
```

Adoption requires the exact root of a checked-out branch registered to the canonical repository. Its HEAD must contain the same pinned base commit, its config and Compose contract must match that commit, and it must not contain pre-existing `.stackstead` tool state. The ancestry check is repeated on later lifecycle operations. Stackstead then creates runtime state and generated contract files but records `source_ownership: external`. All later operations require the external checkout's pointer to reciprocally identify the same manifest, preventing one adopted manifest from being redirected to another checkout. Destroy removes the exact Compose project, volumes, Stackstead state, and generated `.stackstead` directory while preserving the manager-owned checkout. See [manager integrations](integrations.md).

## Start

```sh
stackstead up feature-a
```

`up` locks the stackstead, regenerates the env and Compose ownership contracts, runs configured dependency/link setup, runs `pre_up` hooks, verifies or creates the runtime-token claim and rejects foreign resources in the target namespace, starts the exact Compose project from the manifest, verifies the created resources carry the runtime token, waits for configured Postgres reachability, runs a configured seed command and `post_up` hooks, waits for all HTTP/custom health checks, and refreshes manifest status. Failures retain inspectable state and event history.

Successful human output reports timings for the configured phases and the total.
JSON output remains the version 1 `StacksteadChange` document.

Use these commands before editing service startup code or database setup:

```sh
stackstead inspect feature-a
stackstead logs feature-a --tail 200
stackstead db status feature-a
```

## Launch a new environment

```sh
stackstead launch feature-a -- claude
```

`launch` is the new-environment path: it runs `create`, carries the returned full ID
through `up`, then starts the child with `run`. Failed startup leaves the environment
available for inspection. Existing names are rejected rather than reused.

## Inspect and use

```sh
stackstead ps
stackstead inspect feature-a
stackstead env feature-a
stackstead context feature-a --print
stackstead open feature-a web
stackstead logs feature-a --service web --follow
stackstead run <full-id> -- npm test
stackstead exec <full-id> web -- nginx -t
```

Structured commands accept `--json`; every document has command-owned `kind` and `version` fields and never serializes a persistence type directly. `StacksteadInspection` version 3 separates the recorded manifest status, live observations, and an `effective` status with its basis and observation timestamps. The remaining current documents are version 1. Lifecycle mutations return a `StacksteadChange` envelope with an `action` and a `stackstead` view. `run`, `exec`, `launch`, and `logs --follow` reject JSON because they own stdout, while `destroy --json` requires `--yes` to prevent prompt output from contaminating JSON. `stackstead open ... --print` returns a configured URL without launching a browser. `stackstead env --print` redacts secret-like values and credential-bearing DSNs unless `--show-secrets` is explicitly supplied.

## Stop

```sh
stackstead stop feature-a
```

Stop verifies the claim and resource ownership before delegating to `docker compose stop`. It preserves the branch, worktree, generated contract, event history, containers, claim, and Compose volumes so work can resume with `up`. If no claim or runtime resources have ever existed, stop is an idempotent no-op.

## Repair

```sh
stackstead doctor
stackstead doctor --fail-on-error
stackstead repair feature-a
```

`doctor` is read-only. It checks local prerequisites, the policy marker in a
root `AGENTS.md` or `CLAUDE.md`, config, state/manifests, typed event journals,
lock files, port allocation, Compose port patterns, generated files, worktree
discovery, and known Docker projects. Missing, unversioned, outdated, or newer
policy markers are warnings. The default command reports the complete result
and exits successfully when diagnostics were produced. `--fail-on-error` exits
1 only when at least one error diagnostic exists, making the same complete
human or JSON report suitable for CI; warnings alone still exit 0.

`repair` is deliberately conservative. It first verifies the exact host-wide port lease, then may regenerate env, context, and pointer files; recreate non-destructive state directories; refresh the Git exclude; rerun configured dependency/link setup; and refresh status. It does not delete worktrees or volumes, rewrite Compose files, or run Docker prune.

## Destroy

```sh
stackstead destroy feature-a
# non-interactive only after reviewing the target
stackstead destroy feature-a --yes
```

Destroy shows the resources it will remove and requires confirmation by default. It validates the manifest, state-root containment, worktree ownership, and Compose identity before it:

1. Refuses a dirty worktree before deleting runtime data.
2. Runs `pre_destroy` hooks and verifies the worktree remains clean.
3. Stops the verified runtime and runs Compose `down -v --remove-orphans --rmi local` for the manifest's durable project identity only. This removes Compose-local build images without a custom image tag while preserving pulled and explicitly tagged images. A never-started stackstead with no claim or candidate resources skips Compose and image removal.
4. Removes a Stackstead-owned Git worktree, or only the generated `.stackstead` directory for externally owned source. If normal removal of Stackstead-owned source fails, it uses a pinned minimal container against only that exact worktree to restore host ownership and permissions, then retries. The helper verifies the runtime-token claim; external source and ordinary removable worktrees never invoke it.
5. Removes the validated stackstead state directory after the mutation and agent-run locks have protected the full teardown.

Before runtime removal, Stackstead writes an identity-bound `teardown.json`. It has only three phases: `runtime_remove`, `source_remove`, and `finalize`. A retry resumes the recorded phase, so a failed Compose removal does not rerun `pre_destroy` and a completed runtime removal is not repeated while source cleanup is retried. Runtime ownership is rechecked before mutation, the claim is retained until source cleanup succeeds, and the exact global port lease is released only during finalization. Other lifecycle mutations reject an environment with active teardown state; `inspect` remains available and reports the failed or incomplete phase. There is no compatibility bridge for earlier experimental teardown journals.

Stackstead never performs global Docker pruning and never treats an arbitrary directory as a destroy target. Destruction is permanent for stackstead-local Compose volumes. Commit or export anything valuable first.

Destroy preserves the Git branch because it may contain committed user work and
retains the project coordination lock as stable concurrency state. These are not
orphaned environment resources and should not be removed as routine cleanup.

Inspection and cleanup validate the durable manifest independently of later non-destructive config path changes, so a revised `env.file` or context path cannot strand an older runtime. Regenerating operations such as `up`, `repair`, `run`, and `exec` additionally require the current config to match the manifest contract.

JSON-mode destruction requires `--yes`; this prevents an interactive prompt from corrupting machine-readable stdout.

## Concurrency and events

Create uses a project-level file lock for project state and a per-user port-lease lock for allocation across every Stackstead project. State roots are resolved through their deepest existing ancestor and rejected if the physical result is inside the repository; Unix lock opens additionally do not follow the final component. Creation holds the new stackstead lock before persisting its pending manifest and through `post_create`. Mutating operations such as `up`, `repair`, and `destroy` acquire only pre-existing lock files, so a delayed command cannot recreate state after teardown. Missing mutation, run-lease, or port-lease ownership is a contract error and is not reconstructed. Contended file locks wait for at most 30 seconds, allowing ordinary parallel agents to serialize without waiting forever. `run` and `exec` hold a shared run lease for the active command, so destructive or mutating lifecycle work waits for it to finish. The Unix `run` supervisor releases that lease only after host-child cleanup; the foreground Compose client inherits the `exec` lease. Destroy requires the lease exclusively.

`state/events.jsonl` contains `StacksteadEvent` version 1 JSON lines. Event type and status are closed enums, each append is one newline-terminated write followed by a data sync, and command output is redacted where appropriate. The log is recovery and diagnostic state, not an audit service.
