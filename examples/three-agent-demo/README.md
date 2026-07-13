# Three-agent state-isolation proof

This is a reproducible failure-and-recovery demonstration, not a slideware
example. Three Stackstead branches run the same Nginx/Postgres application at the
same time. Each branch changes migration ID `202607090001` to a conflicting
payload and seeds account `1` with a different owner. Those changes can coexist
only if the agents have different databases.

Prerequisites are `stackstead`, Git, Docker Compose, `jq`, and `curl`. Container
images may be pulled on the first run. The demo never runs Docker prune and
never constructs cleanup targets from branch names.

## One command

From the Stackstead repository:

```sh
examples/three-agent-demo/demo.sh all /tmp/stackstead-three-agent-proof
```

Set `STACKSTEAD_BIN=/absolute/path/to/stackstead` when the binary is not on `PATH`.
The command copies this example to a fresh Git repository, creates and starts
`alpha`, `beta`, and `gamma`, verifies their identities and state, kills only
beta's Postgres container, proves alpha and gamma still work, recovers beta with
its data intact, creates then retires an alpha Compose service, and proves exact
teardown removes that orphan before destroying the three ledger identities.

If an assertion fails, `all` stops and retains the ledger, manifests, containers,
and event logs. Inspect them, then run the copied repository's `demo.sh cleanup`;
the script does not hide a failed phase with automatic teardown.

The copied Git repository remains for inspection; Stackstead source worktrees,
Compose projects, and volumes are removed. Delete the copy yourself only after
reviewing its path.

## Inspect every phase

Use separate commands when you want to inspect the retained failure:

```sh
examples/three-agent-demo/demo.sh prepare /tmp/stackstead-three-agent-proof
/tmp/stackstead-three-agent-proof/demo.sh create
/tmp/stackstead-three-agent-proof/demo.sh verify
/tmp/stackstead-three-agent-proof/demo.sh crash

# Inspect manifests, events, and `docker compose ps` here.
/tmp/stackstead-three-agent-proof/demo.sh recover
/tmp/stackstead-three-agent-proof/demo.sh verify
/tmp/stackstead-three-agent-proof/demo.sh orphan
/tmp/stackstead-three-agent-proof/demo.sh cleanup
```

`cleanup` reads `.demo-stacksteads.tsv`, validates each manifest ID, and invokes
`stackstead destroy <full-id> --yes`. It checkpoints each completed ledger row,
so a partial failure is restartable. It refuses a missing or mismatched manifest
while that exact project's resources remain.
Stackstead in turn refuses dirty worktrees and targets only the manifest's Compose
project and volume. Re-running cleanup after success is a no-op.

## Expected assertions

- Three manifest IDs, Compose projects, worktrees, and pairs of host ports are
  distinct.
- Every published Docker port binds only to loopback, never all host interfaces.
- `inspect` reports the successful one-shot setup service as `completed (0)`.
- Each Postgres container's Compose label equals its manifest project.
- The same migration ID has the branch-specific payload in each database.
- The same seed key has `alpha`, `beta`, or `gamma` as its branch-local value.
- Each generated web URL returns the example page.
- Killing beta by its exact Compose identity does not affect alpha or gamma.
- `stackstead up` recovers beta under the same identity and preserves beta's row.
- Exact teardown removes a project-labeled service retired from the current Compose file.
- Cleanup passes only full ledger IDs to Stackstead and never calls broad prune.

Run the demo against a quiet local Docker daemon if you also want to compare
global Docker inventory without unrelated concurrent changes.
