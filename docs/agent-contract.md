# Agent contract

Use command-owned CLI JSON for automation and generated `AGENT_CONTEXT.md` for agent instructions. Both identify the same environment. The manifest and pointer sections below explain persisted state.

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

- Kind and version
- Stackstead ID, slug, cryptographically random short ID, runtime ownership token, project, branch, pinned base commit, and explicit source ownership
- Canonical repository, project-state, stackstead, worktree, state, and per-user port-lease registry paths
- Compose project identity and resolved Compose file paths
- Service-to-host-port, container-port, and service-to-URL maps
- Generated env, context, pointer, and event-log paths plus generated env key names
- Source, dependency, runtime, database, and health status
- Readiness declaration and its resolved Compose requirements
- Optional Postgres seed metadata
- Creation and update timestamps

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

## Process boundaries

Stackstead also supplies direct host and service process boundaries:

```sh
stackstead run feature-a -- claude
stackstead run feature-a -- <agent-or-command> [arguments...]
stackstead exec feature-a api -- <command> [arguments...]
```

`run` starts the child in the exact recorded checkout with generated environment plus pinned Stackstead and Compose identity. `exec` starts the command in the exact configured, owned, running Compose service. Both commands return the child exit status. See [Agent integration](agent-integration.md).

## Agent context Markdown

The generated worktree contains `.stackstead/AGENT_CONTEXT.md`. It identifies the exact stackstead, branch, source path, Compose project, URLs, ports, environment file, manifest, event log, database service/strategy/endpoint/name, project rules, and exact full-ID commands for inspection, logs, recovery, and teardown. Credentials remain in the generated environment and are not copied into the context.

An agent should begin runtime-sensitive work by reading it:

```sh
stackstead context feature-a --print
stackstead inspect feature-a
```

The expected operating rules are simple:

1. Use only the source checkout named in the context.
2. Use only its generated ports, URLs, and env file.
3. Do not fall back to a shared development database.
4. Inspect status and logs before changing startup code.
5. Check database reachability before applying migrations.
6. Stop or destroy only through the named stackstead contract.

Repositories can append specific rules through `agent.rules` in `stackstead.yaml`.

## Pointer file and discovery

`.stackstead/stackstead.json` is a small, non-secret pointer containing the stackstead ID, project identity, and absolute locations of the manifest, repository root, project state root, and stackstead root. It lets commands issued anywhere under the generated source checkout find the original contract without recalculating `state.root` from a copied config.

The pointer is not the manifest and is not authority to delete a path. Stackstead reads and writes only pointer version 2; the authoritative manifest is version 3. Earlier pointer versions are rejected, including by `repair`; destroy those environments with the binary that created them, then recreate them with the current binary. Every resolved operation requires the pointer and manifest to reciprocally agree on full ID, project, repository, state root, stackstead root, manifest path, and the exact lexical location from which discovery found the pointer. Destructive operations additionally validate containment, source ownership, branch binding, locks, the runtime ownership token, and the exact Compose identity.

## Generated environment

`.stackstead/.env` binds template output such as `WEB_PORT`, `DATABASE_URL`, and `STACKSTEAD_ID` to this stackstead. It starts with a generated-file warning and has deterministic key ordering.

`stackstead env feature-a` reports its location and redacted keys. `stackstead env feature-a --print` remains redacted unless the user explicitly adds `--show-secrets`. Agents should not paste its contents into logs or durable knowledge.

## Compose runtime boundary

Compose-managed volumes use project-scoped names for separate environment data. Stackstead rejects custom `name` values on managed volumes and networks before startup, not only on collision. External resources and shared host bind mounts are accepted but can share state, as can host networking and services outside Stackstead. Document shared-state decisions in `agent.rules`; see [Compose isolation](compose.md#volume-isolation).

Manifest v3 also binds Docker resources to `runtime_token`. Stackstead appends a generated ownership override to every Compose command, exposes that final file in JSON `compose_files`, labels each direct service and managed network or volume, and maintains a deterministic claim volume for the Compose namespace. Startup, stop, and destructive teardown enumerate project-labeled and exact-name resources and reject missing or mismatched ownership before Compose runs. Teardown re-enumerates resources before releasing the claim. A never-started stackstead with no claim and no candidate resources remains safe to stop or destroy as a no-op.
