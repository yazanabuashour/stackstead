<!-- stackstead-policy: 1 -->
# Stackstead Project Rules

Stackstead is a single Rust command-line interface that gives parallel coding
agents isolated Git worktrees, Docker Compose runtimes, ports, and data under
one durable identity. It has no daemon, server, plugin loader, or hidden
database; lifecycle commands fail closed when persisted ownership cannot be
proved.

## Code Map

- `src/main.rs` and `src/cli/` define the process entry point, command surface,
  dispatch, presentation, and runtime wiring.
- `src/lifecycle/` sequences create, adopt, up, stop, inspect, and destroy work.
- `src/config/`, `src/manifest/`, `src/state.rs`, and `src/discovery.rs` own
  configuration, durable contracts, state roots, and identity lookup.
- `src/paths/`, `src/lock/`, `src/lease/`, and `src/ports/` enforce filesystem,
  concurrency, lease, and allocation safety.
- `src/compose/` owns Compose planning, supported rewrites, runtime control,
  resource naming, and ownership checks; `src/command/` runs and redacts
  external commands.
- `src/output/`, `src/context.rs`, and `src/agent/` own versioned JSON, generated
  agent context, and host-command execution.
- `src/doctor/`, `src/repair.rs`, `src/health.rs`, `src/database.rs`, and
  `src/events/` cover diagnostics, recovery, readiness, database support, and
  lifecycle receipts.
- `tests/cli_acceptance.rs` and `tests/cli_acceptance/` hold command-level
  contracts; focused module tests live beside implementation. `scripts/ci.sh`
  is the project-owned readiness entry point, and `docs/` is the operator and
  integration reference.

## Runtime Identity And Safety

- Treat generated Stackstead state as authoritative. Read `STACKSTEAD_CONTEXT` or
  use `stackstead context`/`stackstead inspect`; never guess ports, URLs, database
  names, Compose project names, worktree paths, or teardown targets.
- Prefer `stackstead run <full-id> -- <command>` for commands that must execute in
  an environment. The `--` boundary is required. `run --json` is intentionally invalid
  because the child owns stdout and stderr.
- Capture the durable full `stackstead_id` from JSON output and use it in scripts,
  integrations, and destructive or runtime-sensitive commands. Slugs are only
  an interactive convenience; inside an environment, use `$STACKSTEAD_ID` directly.
- Confirm the active directory is `$STACKSTEAD_WORKTREE`. Do not edit generated
  `.stackstead` files or print, copy, or retain `.stackstead/.env`; use
  `stackstead repair` for regeneration. `repair` and `up` may rerun dependency,
  database, and hook work and are not read-only.
- Use Stackstead lifecycle commands instead of bare `docker compose`. Never use a
  global Docker prune or broad name-based cleanup. `stop` preserves source and
  volumes; run `destroy --yes` only for the exact full ID after identity and
  dirty-check validation. Do not bypass fail-closed ownership or manifest errors.

## Configuration And Contracts

- `stackstead compose plan` is read-only. Use `stackstead compose apply --yes` to
  make supported edits, inspect the diff, and commit `stackstead.yaml` plus its
  Compose file before `create`, `adopt`, or `up`; environments pin committed `source.base`.
- Do not add a plugin abstraction, compatibility layer, or migration path
  without a concrete current use case. This pre-release codebase prefers
  explicit breaking cleanup over silent inference.
- CLI JSON is a versioned transport contract, not a serialized manifest or
  internal type. Prefer `stackstead --json <subcommand> ...`, keep command-owned
  DTOs, validate `kind` and `version` (and mutation `action`), and wrap lists.
  Commands that stream or prompt may reject JSON or require noninteractive flags.
- Invoke manager hooks from trusted installed absolute paths, never from a
  branch-controlled worktree. Teardown belongs in blocking pre-remove handling
  and requires the documented explicit teardown authorization.

## Verification

- Run `scripts/ci.sh` for the full local Linux sequence used by CI. Use its
  `rust`, `docker`, or `macos` mode for the corresponding focused job.

- For `scripts/docker-integration.sh`, the preceding debug build may instead be
  replaced by setting `STACKSTEAD_BIN` to an explicit executable. Docker and the
  Compose plugin are required. Lifecycle, isolation, and teardown claims need
  the live Docker integration; mocks alone are insufficient.
- Changes to manifests, pointers, generated context, CLI/JSON, config, ports,
  Compose identity, locks, or teardown need both happy-path and ambiguous,
  tampered, or corrupt-state regression coverage.
- During checkpoint review, request `api-compat` for
  CLI/config/schema/generated-output contracts and `concurrency` for lifecycle
  races; request no other focused reviews. If checkpoint tooling does not
  support a required focused review, report that review as unavailable.
