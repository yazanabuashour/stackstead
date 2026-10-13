# Agent contract

Use command-owned CLI JSON for automation, not the manifest. This reference
covers response envelopes and persisted identity. For host/service execution,
generated context, and injected environment, see [Agent integration](agent-integration.md).

## Current worktree identity

From any directory beneath a generated worktree, resolve its validated full ID
before composing another command:

```sh
id="$(stackstead current)"
stackstead run "$id" -- true
```

Plain output is exactly the full `stackstead_id` plus one newline.
`stackstead --json current` returns a `StacksteadCurrent` version 1 object with
exactly `kind`, `version`, `stackstead_id`, `source_ownership`, `repo_root`,
`worktree`, and `pointer`. Consumers must validate `kind` and `version` before
using the other fields.

`current` works only through generated worktree discovery. It validates the
manifest's durable layout, exact reciprocal pointer path, registered Git branch,
and pinned base commit. It also requires the manifest's project and state root
to match `stackstead.yaml` in Git's primary worktree, so branch-written state
cannot select an unrelated registered identity. It does not read generated
environment, inspect Docker, or report mutable runtime status. An active
teardown journal does not by itself block identity lookup, but the worktree
pointer and primary project configuration must still exist. `destroy --yes`
still owns mutation safety.

## CLI JSON

Use `stackstead --json …` for automation. Although the argument parser also
accepts the global `--json` flag after a subcommand, consumers should use the
canonical form shown here. CLI JSON is a versioned command contract, not a
serialized manifest or internal type.

| Invocation | Top-level `kind` | `version` | Mutation `action` |
| --- | --- | --- | --- |
| `stackstead --json init [--compose-file <repo-relative-path>]` | `StacksteadInit` | `"1"` | — |
| `stackstead --json compose plan [--compose-file <repo-relative-path>]` | `ComposePlan` | `"1"` | — |
| `stackstead --json compose apply --yes [--compose-file <repo-relative-path>]` | `ComposeApply` | `"1"` | — (no `action` field) |
| `stackstead --json create <name>` | `StacksteadChange` | `"1"` | `created` |
| `stackstead --json adopt <name> --worktree <absolute-path>` | `StacksteadChange` | `"1"` | `adopted` |
| `stackstead --json up <full-id>` | `StacksteadChange` | `"1"` | `started` |
| `stackstead --json ps` | `StacksteadList` | `"2"` | — |
| `stackstead --json current` | `StacksteadCurrent` | `"1"` | — |
| `stackstead --json inspect <full-id>` | `StacksteadInspection` | `"4"` | — |
| `stackstead --json env <full-id> [--print [--show-secrets]]` | `StacksteadEnvironment` | `"1"` | — |
| `stackstead --json logs <full-id> [--service <service>] [--tail <lines>]` | `StacksteadLogs` | `"1"` | — |
| `stackstead --json context <full-id> [--print]` | `StacksteadContext` | `"1"` | — |
| `stackstead --json open <full-id> [service] [--print]` | `StacksteadOpen` | `"1"` | — |
| `stackstead --json db status <full-id>` | `DatabaseStatus` | `"1"` | — |
| `stackstead --json stop <full-id>` | `StacksteadChange` | `"1"` | `stopped` |
| `stackstead --json destroy <full-id> --yes` | `StacksteadChange` | `"1"` | `destroyed` |
| `stackstead --json doctor [--fail-on-error]` | `DoctorReport` | `"1"` | — |
| `stackstead --json repair <full-id>` | `StacksteadChange` | `"1"` | `repaired` |

### Consume responses safely

- Every successful response is a top-level object with string `kind` and
  `version` fields. Validate both before reading command-specific fields.
  `StacksteadInspection` uses version `"4"`, `StacksteadList` uses `"2"`, and
  every other current response uses `"1"`.
- Only `StacksteadChange` has an `action` field. Validate its exact value before
  acting on `stackstead`. The supported values are `created`, `adopted`,
  `started`, `stopped`, `destroyed`, and `repaired`.
- After validating `kind: "StacksteadChange"`, `version: "1"`, and action
  `created` or `adopted`, read `.stackstead.stackstead_id` from the corresponding
  `create` or `adopt` response. Use that full ID for later automation, not a slug.
  `current` instead exposes `.stackstead_id` at the top level after validation
  of `kind: "StacksteadCurrent"` and `version: "1"`.
- No command returns a bare top-level array. The collection fields are
  `ComposePlan.ports`, `ComposePlan.warnings`, `StacksteadList.stacksteads`,
  `StacksteadInspection.live.services`, `StacksteadInspection.warnings`, and
  `DoctorReport.diagnostics`; stackstead views also contain `compose_files`.
  Live service collections are nullable when observation fails.
- `run`, `exec`, and `launch` always reject JSON because the child owns stdout
  and stderr. `logs --follow` rejects JSON because it streams. Public help still
  shows the global option for these commands, so help output alone does not
  establish JSON compatibility.
- JSON `destroy` requires `--yes` and fails with empty stdout before it can
  prompt when that flag is absent. Plain `destroy` prompts. `compose apply` also
  requires `--yes`, but it never prompts.
- `env` redacts values whose case-insensitive key contains `PASSWORD`, `TOKEN`,
  `SECRET`, `KEY`, `CREDENTIAL`, or `AUTH`. It also redacts URLs with nonempty
  user information and a host around `@`. `--show-secrets` is valid only with
  `--print`; in JSON mode that combination puts unredacted values in `values`,
  while `--print` alone remains redacted.
- JSON `open` never launches a browser and always reports `opened: false`;
  `--print` does not change that behavior. `context --print` includes `content`,
  while `content` is `null` without `--print`.
- `doctor --fail-on-error` emits the complete `DoctorReport`, then exits 1 only
  if it contains an error diagnostic. Runtime command failures have no JSON
  error envelope: Stackstead writes the error to stderr and exits 1. Argument
  usage errors, including `--show-secrets` without `--print`, exit 2.

The [generic creation wrapper](../integrations/generic/create-stackstead-owned.sh)
validates the create envelope and extracts the nested ID before calling `up`.
Its output is a flattened projection of `.stackstead`, so the wrapper's
`.stackstead_id` is not the field path in raw `create` JSON.

### Inspection version 4 and list version 2

Inspection keeps recorded component status under `stackstead.status`. Under
`live`, runtime activity, declared readiness, and application health answer
separate questions:

| Field | Meaning |
| --- | --- |
| `runtime.status` | Runtime component status, separate from readiness. |
| `runtime.running` | Whether any observed container is running; `null` when unavailable. |
| `runtime.activity` | `active`, `inactive`, or `unknown`. |
| `services` | Owned container rows, or `null` when capture is unavailable. An empty array means a successful empty capture. |
| `readiness.status` | `unconfigured`, `ready`, `not_ready`, or `unknown`. |
| `readiness.required` | Per-service requirement results. |
| `readiness.issues` | Reasons readiness could not be established. |

Each service row contains `service`, `container`, immutable `id`, raw `state`,
formatted `status`, `exit_code`, `health`, `healthcheck_enabled`, `oneoff`,
`container_number`, and `config_hash`. Unavailable optional metadata is `null`.
An exited-zero container has status `exited (0)`; only explicit job intent makes
that successful readiness evidence.

Each requirement result contains `service`, `role`, `expected_instances`,
`observed_containers`, `satisfied_instances`, `status`, and `issues`.
`expected_instances` is `null` when current requirements cannot be established.
Counts do not replace the status or issues. Optional service failures remain in
service rows without alone blocking the required set. See [readiness evidence](compose.md#readiness-evidence)
for ownership, healthcheck, replica, and fingerprint constraints.

`StacksteadList.stacksteads` entries retain `stackstead_id`, `branch`, `ports`,
and `worktree`. Their `runtime` is the activity string; `readiness` and nullable
`services` have the same shapes as inspection, and `issues` records observation
problems. `ps` runs no application probes.

Inspection retains `live.database`, `live.health`, `effective`, and `warnings`.
Each effective component reports `status` and `basis`; `effective` also contains
`phase`, `recorded_at`, and `observed_at`. Basis is `live`, `recorded`, `lifecycle`,
or `unconfigured`. Empty application checks yield `effective.health.status` of
`unknown` with basis `unconfigured`, not positive health evidence. Command-backed
health uses recorded status when the runtime is running; passive inspection
never executes custom checks. Runtime readiness does not replace that result.

Service rows are sorted deterministically. List and inspection assemble separate
observations, not an atomic snapshot or a guarantee that a service remains ready
after the command returns. Preserve `null` rather than treating unavailable
observations as stopped, empty, or healthy.

## Manifest JSON

Every stackstead has a durable manifest at `<stackstead-root>/state/manifest.json`. Lifecycle operations use it as authoritative state. Routine integrations should consume command-owned CLI JSON rather than parse the manifest directly.

The manifest contract is version 3. It requires explicit `source_ownership`, a
cryptographically random runtime ownership token, a Compose project equal to
`<project>-<stackstead-id>`, pre-existing mutation and run-lease lock files, and a
mandatory `readiness` object. Unknown fields and missing or corrupt readiness
contracts fail closed. Readers validate the contract header before interpreting
the body.

The field definitions live in [`StacksteadManifest`](../src/manifest.rs) and
[`readiness::Contract`](../src/readiness/model.rs). These are persisted internal
contracts, not Rust SDK types or a substitute for command-owned output.

`readiness.configuration` is `unconfigured` or `declared`. A declared contract
has a nonempty exact service-to-role `required` map. Its optional `resolved`
object stores `profiles`, the full normalized-model `model_hash`, and `services`
with each required service's `replicas` and native `config_hash`. Resolved service
keys must match the declaration exactly, counts must be positive, and both hash
types use lowercase SHA-256 hex. Unresolved declarations cannot be ready.

The manifest records where generated environment lives, but it does not copy
environment values that may contain secrets. It retains the raw non-secret
startup profile selection, not the normalized Compose model. Writes are atomic
where practical.

## Pointer file and discovery

`.stackstead/stackstead.json` is a small, non-secret pointer containing the stackstead ID, project identity, and absolute locations of the manifest, repository root, project state root, and stackstead root. It lets commands issued anywhere under the generated source checkout find the original contract without recalculating `state.root` from a copied config.

The pointer is not the manifest and is not authority to delete a path. Stackstead reads and writes only pointer version 2; the authoritative manifest is version 3. Earlier pointer versions are rejected, including by `repair`; destroy those environments with the binary that created them, then recreate them with the current binary. Every resolved operation requires the pointer and manifest to reciprocally agree on full ID, project, repository, state root, stackstead root, manifest path, and the exact lexical location from which discovery found the pointer. Destructive operations additionally validate containment, source ownership, branch binding, locks, the runtime ownership token, and the exact Compose identity.

Docker resources are independently bound to the manifest's `runtime_token`;
see [Compose ownership checks](compose.md#project-identity). Filesystem identity
does not prove resource ownership or [isolation from external state](compose.md#volume-isolation).
