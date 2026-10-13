# Rust architecture

Stackstead is one Rust CLI crate, with no daemon, server, plugin loader, or
hidden database. Use the [code map](../AGENTS.md#code-map) to find a subsystem and
[Contributing](../CONTRIBUTING.md) to run its checks. This page explains the
invariants that changes must preserve, not the current file split.

## Keep contracts separate

Configuration declares the repository's runtime; the durable manifest records
one environment's identity and allocated resources. Lifecycle code uses the
manifest for worktree paths, Compose identity, ports, URLs, and generated files
instead of rediscovering them from ambient state.

CLI JSON is a separate automation contract, not a serialized manifest. The
sealed marker in [`output.rs`](../src/output.rs) permits only command-owned
response types to reach the JSON writer. Adding a serializable internal field
must not add it to stdout. See the [agent contract](agent-contract.md) for
transport versions and persisted-state constraints.

The crate is binary-only and has no supported Rust SDK. Introduce a library or
adapter abstraction only when concrete callers establish a durable boundary;
internal modules are not compatibility promises. The supported providers are
explicit configuration values, not interchangeable plugins.

## Preserve identity before mutation

The generated pointer binds a worktree to its original repository and manifest.
Recalculating `state.root` from a copied config would resolve it relative to the
wrong checkout. Discovery checks the pointer and manifest reciprocally; the
pointer alone is never deletion authority.

[`paths`](../src/paths.rs) owns checked filesystem deletion.
[`lifecycle/validation.rs`](../src/lifecycle/validation.rs) binds durable layout,
source ownership, branch, and Compose identity. `--yes` skips confirmation, not
validation. Manifest writes derive their destination from the manifest's state
directory and use atomic persistence; generated secrets stay out of the manifest.

Docker ownership is separate from filesystem containment. The Compose adapter
labels managed resources with the manifest's runtime token, maintains a claim
volume, and checks project-labeled and exact-name resources before mutation.
Unsupported resource shapes fail closed rather than starting without complete
ownership metadata. See [Compose identity](compose.md#project-identity) and
[teardown recovery](lifecycle.md#destroy).

## Hold access across lifecycle work

[`lifecycle/access.rs`](../src/lifecycle/access.rs) acquires the mutation lock,
then the run lease, then reloads the manifest. Its `HeldEnvironment` retains the
guards and manifest together and rejects a changed full ID, runtime token, or
state directory after waiting. Use it rather than reassembling the protocol.

Creation's `CreateOutcome` privately retains its mutation lock. `launch` transfers
that outcome into startup, then transfers held access into agent execution.
Converting startup's exclusive run lease to shared access retains the mutation
lock across the conversion. Agent execution releases mutation access only after
its checks and transfers the shared lease to child supervision or foreground
Compose execution.

Held access is not proof of a valid runtime contract. Repair may restore a
missing pointer, stop uses the durable contract, and startup and agent execution
require the current contract. Destroy keeps its separate recovery-aware locking
sequence. Missing locks and port leases are errors, not state to reconstruct.
See [concurrency and events](lifecycle.md#concurrency-and-events).

## Keep runtime evidence distinct from authority

[`compose/snapshot.rs`](../src/compose/snapshot.rs) interprets owned container
observations. Startup uses the snapshot directly; readiness evaluation and
output conversion read its container evidence.

A snapshot is neither an atomic Docker view nor ongoing mutation authority.
Unavailable evidence is not an empty runtime. Activity, declared readiness, and
application health answer separate questions: completed jobs can be ready while
inactive. Inspection retains manifest-change detection; startup retains
model-drift rejection and its shared verification deadline. See [readiness
evidence](compose.md#readiness-evidence) and [inspection fields](agent-contract.md#inspection-version-4-and-list-version-2).

## Keep process boundaries explicit

Built-in Git and Docker operations use executable/argument arrays. Configured
dependency, hook, seed, and health commands share a command shape: `shell: false`
parses an executable and arguments; `shell: true` explicitly opts into the
platform shell. Generated values are not concatenated into shell strings.

The command adapter owns capture, diagnostics, and secret redaction. Host-agent
supervision and foreground Compose execution own the active run lease; their
cleanup guarantees differ by platform. Preserve the [documented process
boundary](agent-integration.md), including its failure cases, when changing either
path. Mocks do not establish live lifecycle, isolation, or teardown safety; those
claims require the [Docker-backed checks](../CONTRIBUTING.md#required-checks).
