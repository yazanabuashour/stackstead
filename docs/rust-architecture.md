# Rust architecture

Stackstead is a single Rust CLI crate. Its architecture separates configuration and durable state from external command adapters, while keeping lifecycle sequencing explicit. There is no daemon, server, plugin loader, or hidden database.

## Module layout

```text
src/
  main.rs       CLI entry point
  cli.rs        clap command and flag definitions
  output.rs     versioned command-owned JSON response DTOs
  error.rs      typed domain errors
  config.rs     YAML types, parsing, defaults, validation
  manifest.rs   versioned manifest types and atomic persistence
  state.rs      project/stackstead state roots and lookup
  discovery.rs  config and pointer-file discovery
  lock.rs       project and stackstead file locks
  paths.rs      canonicalization, containment, destroy safety
  slug.rs       stackstead name sanitization and ID generation
  ports.rs      deterministic port slots and bind probes
  template.rs   small fail-closed {{ key }} interpolation
  command.rs    process execution, capture, diagnostics, redaction
  git.rs        local Git worktree operations
  compose.rs    Compose discovery, narrow rewrites, runtime commands, port checks
  envfile.rs    deterministic generated environment files
  context.rs    generated AGENT_CONTEXT.md
  agent.rs      exact stackstead command/agent execution boundary
  database.rs   Postgres reachability and seed state
  health.rs     generic loopback HTTP and custom-command readiness
  lifecycle.rs  create/up/stop/destroy orchestration
  doctor.rs     read-only diagnostics
  repair.rs     conservative regeneration
  events.rs     append-only JSONL events
  open.rs       manifest URL selection and opening
```

The exact file split may stay smaller while implementation is compact. The important boundaries are configuration, persisted contracts, safety, external processes, and lifecycle orchestration—not one file per abstract concept.

## Command runner

Built-in Git and Compose operations use `std::process::Command` with an executable and argument array. The runner captures stdout and stderr, includes useful command context in errors, records selected lifecycle boundaries, and redacts values associated with obvious secret names.

Repository-configured dependency commands, hooks, link commands, and seed commands use one shared command shape. `shell: false` is parsed into an executable and arguments. `shell: true` is an explicit project opt-in to platform-shell behavior. Stackstead-generated values are not concatenated into shell strings.

## Durable manifest

`manifest.json` is the primary runtime contract. Versioned serde structs keep its shape explicit; CLI JSON is a separate transport contract made from owned response DTOs. Lifecycle code reads the manifest for worktree paths, Compose identity, ports, URLs, and generated files instead of rediscovering them from ambient state.

Lifecycle manifest writes derive their destination from the manifest's canonical state directory, then use a temporary file, flush, and rename where practical. There is no path-taking compatibility writer or draft-format migration. Secret values stay in the generated env file rather than being copied into the manifest.

## Rust and JSON surfaces

Stackstead ships one binary crate (`autolib = false`, `publish = false`) and intentionally has no supported Rust SDK yet. Internal modules may change together without creating a premature semver contract. A library should be introduced only when concrete embedding use cases establish a smaller durable API than the application internals.

Machine-readable output is the supported automation surface. A sealed internal marker permits only command-owned DTOs to reach the JSON writer. Every response is a top-level object with a semantic `kind` and `version`; lists are wrapped rather than emitted as bare arrays. `StacksteadInspection` version 3 separates recorded, live, and effective status while the remaining current responses are version 1. DTO conversions copy only intended integration data, so manifest fields, doctor internals, and Compose implementation types cannot leak into stdout merely by gaining a serialization derive.

## Pointer-file discovery

Discovery has two deterministic paths:

1. Inside generated source, locate `.stackstead/stackstead.json` and follow its absolute manifest/repository pointers.
2. Otherwise, climb parents for `stackstead.yaml` and resolve the canonical repository state root.

The pointer avoids a subtle bug: interpreting `../.stacksteads` relative to the generated worktree would find a different directory than interpreting it at the original repository. Pointer contents remain non-secret. Every normal resolution validates that pointer and manifest reciprocally identify the same full environment; repair alone may recreate a missing pointer after validating the manifest independently.

## Path safety

All destructive operations pass through centralized path checks. Safety invariants include:

- Reject empty, relative traversal, root, or otherwise dangerous stackstead targets.
- Canonicalize existing ancestors before containment comparisons.
- Require the manifest identity to match the resolved stackstead.
- Require the source/state path to belong to the configured Stackstead root or another explicitly recorded safe location.
- Require Compose project identity to match the manifest.
- Keep `remove_dir_all` behind the checked deletion boundary.
- Never run broad Docker prune commands.

The pointer file alone is not deletion authority. `--yes` skips an interactive prompt; it does not skip safety checks.

## Locks and allocation

A project lock serializes project-local state and index changes. A host-wide per-user lease registry serializes port allocation across repositories and retains exact runtime-token ownership until successful destroy cleanup. A pre-existing stackstead lock serializes lifecycle mutations, and a separate shared/exclusive run lease prevents teardown while an agent child is active. Creation holds the environment lock across first manifest publication and post-create hooks. Contended locks use one bounded 30-second wait. The deterministic allocator checks the lease registry and attempts to bind every candidate port on `127.0.0.1` before committing a slot.

Locks are intentionally simple cross-process file locks. `doctor` diagnoses suspicious lock state rather than implementing automatic lease stealing. `run` retains the shared lease through a private Unix supervisor that binds host-child cleanup to it. `exec` keeps the Compose client in the foreground and hands the lease into that process.

## Reliability scope budget

The four reproduced reliability fixes have a budget of 2,500 net new production
lines, with a hard checkpoint cap of 3,500: teardown 1,100, run supervision 750,
inspection 400, lock waiting 150, and glue 100. General helper supervision,
installer refactors, hook hardening beyond the current JSON contract, journal
compatibility, and recovery for unobserved states are outside this budget. A
future expansion needs a reproduced failure and a regression test rather than
being folded into these mechanisms speculatively.

## Internal adapters

Git worktrees and Docker Compose are small internal modules around command construction and execution. Git distinguishes Stackstead-owned source from explicitly adopted manager-owned worktrees. Compose discovery and rewriting are intentionally narrower than lifecycle execution; structural startup validation binds every published port variable to its manifest allocation. Lifecycle generation appends a last-wins ownership override that labels every directly managed Compose resource with the manifest runtime token. Raw Docker control creates and verifies a deterministic claim volume, enumerates both project labels and exact resource names, and aborts before mutating an occupied namespace; actual Compose subprocesses additionally remove generated key names from the inherited shell, pin the manifest Compose identity, and consume values only from the explicit env file. Postgres support probes the configured host port and runs an optional project seed command; generic health checks cover loopback HTTP statuses and repository commands. Dependency support executes a command or Yarn Classic link workflow.

These boundaries make behavior testable without pretending that providers are interchangeable. Unit and CLI acceptance tests assert command construction, discovery, parsing, ownership, safety, and generated artifacts. The three-agent Postgres/Nginx failure-recovery proof is a mandatory Docker-backed CI job.

## Why there is no plugin architecture

The production contract has one source provider, one runtime provider, one database strategy, and two dependency modes. A public trait/plugin system would add compatibility promises before the boundaries have been proven. Rust enums make supported values and unsupported configuration explicit. New adapters should be added only after repeated real use demonstrates a stable common contract.
