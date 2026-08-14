# Agent contract

Stackstead's agent-native surface consists of one machine-readable artifact and one human-readable artifact, both tied to the same runtime identity.

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
| `stackstead --json ps` | `StacksteadList` | `"1"` | — |
| `stackstead --json current` | `StacksteadCurrent` | `"1"` | — |
| `stackstead --json inspect <full-id>` | `StacksteadInspection` | `"3"` | — |
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
  `StacksteadInspection` alone uses version `"3"`; every other current response
  uses version `"1"`.
- Only `StacksteadChange` has an `action` field. Validate its exact value before
  acting on `stackstead`. The supported values are `created`, `adopted`,
  `started`, `stopped`, `destroyed`, and `repaired`.
- Capture the durable full `stackstead_id` returned by `create` or `adopt` and
  use it for later automation. Do not infer an identity from a slug.
- No command returns a bare top-level array. The collection fields are
  `ComposePlan.ports`, `ComposePlan.warnings`, `StacksteadList.stacksteads`,
  `StacksteadInspection.live.services`, `StacksteadInspection.warnings`, and
  `DoctorReport.diagnostics`; stackstead views also contain `compose_files`.
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

### Inspection version 3

Inspection version 3 keeps recorded status under `stackstead.status`, reports
Compose, database, and passive HTTP observations under `live`, and adds
`effective`. Each effective component has a `status` and a `basis` of `live`,
`recorded`, or `lifecycle`; the envelope includes `phase`, `recorded_at`, and
`observed_at`. Divergence is explicit in `warnings`. A stopped service targeted
by an HTTP check is live-unhealthy even when another service is running. Service
rows remain deterministically sorted. Stackstead is pre-release and does not
emit older inspection versions or provide an output-version switch.

## Manifest JSON

Every stackstead has a durable manifest at `<stackstead-root>/state/manifest.json`. It is the source of truth for lifecycle operations and machine integration.

The only manifest contract is version 2. It requires explicit `source_ownership`, a cryptographically random runtime ownership token, a Compose project equal to `<project>-<stackstead-id>`, and pre-existing mutation and run-lease lock files; unknown fields are rejected. Version 1 lacks the runtime ownership token and is rejected with explicit guidance to destroy it using a compatible older binary and then recreate it. Stackstead is pre-release: it does not infer missing fields, recreate missing ownership state, accept historical custom Compose identities, or silently migrate draft formats. Readers validate the contract header before interpreting the body.

- Kind and version
- Stackstead ID, slug, cryptographically random short ID, runtime ownership token, project, branch, pinned base commit, and explicit source ownership
- Canonical repository, project-state, stackstead, worktree, state, and per-user port-lease registry paths
- Compose project identity and resolved Compose file paths
- Service-to-host-port, container-port, and service-to-URL maps
- Generated env, context, pointer, and event-log paths plus generated env key names
- Source, dependency, runtime, database, and health status
- Optional Postgres seed metadata
- Creation and update timestamps

The manifest records where generated environment lives, but it does not copy environment values that may contain secrets. Writes are atomic where practical.

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

The pointer is not the manifest and is not authority to delete a path. Stackstead writes pointer version 2 and accepts the field-compatible version 1 pointer during transition; the authoritative manifest must still be version 2. Every resolved operation requires the pointer and manifest to reciprocally agree on full ID, project, repository, state root, stackstead root, manifest path, and the exact lexical location from which discovery found the pointer. Destructive operations additionally validate containment, source ownership, branch binding, locks, the runtime ownership token, and the exact Compose identity.

## Generated environment

`.stackstead/.env` binds template output such as `WEB_PORT`, `DATABASE_URL`, and `STACKSTEAD_ID` to this stackstead. It starts with a generated-file warning and has deterministic key ordering.

`stackstead env feature-a` reports its location and redacted keys. `stackstead env feature-a --print` remains redacted unless the user explicitly adds `--show-secrets`. Agents should not paste its contents into logs or durable knowledge.

## Compose runtime boundary

Normal Compose-managed volumes are separated by Compose project name. Explicitly external volumes, host bind mounts, host networking, and services configured outside Stackstead can still share state. A managed globally named volume fails closed on an ownership collision but cannot provide parallel branch-local storage. Treat those as repository-level decisions and document them in `agent.rules`.

Manifest v2 also binds Docker resources to `runtime_token`. Stackstead appends a generated ownership override to every Compose command, exposes that final file in JSON `compose_files`, labels each direct service and managed network or volume, and maintains a deterministic claim volume for the Compose namespace. Startup, stop, and destructive teardown enumerate project-labeled and exact-name resources and reject missing or mismatched ownership before Compose runs. Teardown re-enumerates resources before releasing the claim. A never-started stackstead with no claim and no candidate resources remains safe to stop or destroy as a no-op.
