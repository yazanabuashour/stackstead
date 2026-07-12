# Stackstead Project Rules

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
- Stackstead is intentionally a binary-only product. Do not add a Rust library,
  public module surface, plugin abstraction, compatibility layer, or migration
  path without a concrete current use case. This pre-release codebase prefers
  explicit breaking cleanup over silent inference or backward compatibility.
- CLI JSON is a versioned transport contract, not a serialized manifest or
  internal type. Prefer `stackstead --json <subcommand> ...`, keep command-owned
  DTOs, validate `kind` and `version` (and mutation `action`), and wrap lists.
  Commands that stream or prompt may reject JSON or require noninteractive flags.
- Invoke manager hooks from trusted installed absolute paths, never from a
  branch-controlled worktree. Teardown belongs in blocking pre-remove handling
  and requires the documented explicit teardown authorization.

## Verification

- Treat `.github/workflows/ci.yml` as authoritative. Its current full local
  sequence is:

  ```sh
  cargo fmt --check
  cargo clippy --locked --all-targets --all-features -- -D warnings
  cargo test --locked
  scripts/test-install.sh
  cargo build --locked --release
  scripts/test-release-install.sh target/release/stackstead
  scripts/test-delivery.sh
  cargo build --locked
  scripts/docker-integration.sh
  ```

- For `scripts/docker-integration.sh`, the preceding debug build may instead be
  replaced by setting `STACKSTEAD_BIN` to an explicit executable. Docker and the
  Compose plugin are required. Lifecycle, isolation, and teardown claims need
  the live Docker integration; mocks alone are insufficient.
- Changes to manifests, pointers, generated context, CLI/JSON, config, ports,
  Compose identity, locks, or teardown need both happy-path and ambiguous,
  tampered, or corrupt-state regression coverage.
- When reviewing changes at a checkpoint, use `api-compat` for
  CLI/config/schema/generated-output contracts and `concurrency` for lifecycle
  races; otherwise use no extras.
