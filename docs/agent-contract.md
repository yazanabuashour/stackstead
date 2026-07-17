# Agent contract

Stackstead's agent-native surface consists of one machine-readable artifact and one human-readable artifact, both tied to the same runtime identity.

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

Use stable JSON output for automation:

```sh
stackstead inspect feature-a --json
stackstead ps --json
stackstead db status feature-a --json
```

Agent managers and wrappers can consume these commands without parsing human output. CLI JSON is not the manifest: each command owns a versioned response DTO, so persistence-only fields cannot appear accidentally. Lifecycle mutations use `{ "kind": "StacksteadChange", "version": "1", "action": "...", "stackstead": { ... } }`; inspection uses `StacksteadInspection` version 3, while lists and doctor remain version 1. Consumers should validate `kind` and `version` before reading the command-specific body.

Inspection version 3 keeps recorded status under `stackstead.status`, reports
Compose, database, and passive HTTP observations under `live`, and adds
`effective`. Each effective component has a `status` and a `basis` of `live`,
`recorded`, or `lifecycle`; the envelope includes `phase`, `recorded_at`, and
`observed_at`. Divergence is explicit in `warnings`. A stopped service targeted
by an HTTP check is live-unhealthy even when another service is running. Service
rows remain deterministically sorted. Stackstead is pre-release and does not
emit older inspection versions or provide an output-version switch.

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
