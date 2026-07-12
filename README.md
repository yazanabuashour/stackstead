# Stackstead

**Give every coding agent its own real copy of your application—with separate source, services, ports, and data.**

Stackstead builds on your repository's Docker Compose development setup to give
every coding agent its own worktree, services, ports, and data. Each copy stays
tied to one exact checkout and Compose project, so inspection, recovery, and
cleanup target the right environment.

Stackstead is the runtime substrate under Codex, Claude Code, Cursor, workmux, webmux, Worktrunk, or another manager. It does not replace the agent, terminal, editor, or dashboard.

## Why Stackstead

Starting three application copies is the easy part. The expensive failure is an
agent connecting to the wrong database or cleanup trusting stale, mutable
identity. Stackstead binds source, generated environment, Compose project, ports,
database state, and teardown to one reciprocal manifest and checkout pointer.
Lifecycle commands refuse to proceed when that identity is ambiguous or
redirected.

## Install

Release binaries do not require Rust. The checksummed curl installer, release workflow, supported platform baselines, and generated Homebrew formula are documented in [Installation](docs/install.md). Until `yazanabuashour/stackstead` is published, build locally:

```sh
cargo install --locked --path .
```

Prerequisites are Git and Docker with the Compose plugin. Stackstead v0.1 owns
Compose runtimes; native host processes must be wrapped in Compose or launched
separately with `stackstead run` and managed by their caller.

## Quickstart

### Start an environment

If the repository already contains a reviewed and committed `stackstead.yaml`:

```sh
stackstead launch feature-a -- codex
```

`launch` creates and starts a new environment, then runs the command inside it. It
prints the durable full ID, URLs, and startup timings along the way.

### Add Stackstead

Ask a coding agent to follow the [Stackstead agent setup v1 guide](docs/agent-setup-v1.md):

> Set up Stackstead in this repository. Follow the Stackstead agent setup v1 guide,
> reuse the existing Compose setup, make the smallest changes needed, and show
> me the diff before committing.

Or [set it up manually](docs/quickstart.md). Once the changes are reviewed and
committed, start the first environment with `stackstead launch`.

## What every stackstead owns

- An exact Git worktree and branch, or an explicitly adopted manager-owned checkout
- A collision-resistant ID and Docker Compose project name
- Deterministically allocated host ports and service URLs
- A generated `.stackstead/.env` with redacted inspection commands
- Compose-local containers, networks, volumes, and database state
- A durable, versioned `state/manifest.json`
- `.stackstead/AGENT_CONTEXT.md` generated from the same identity
- Source, dependency, runtime, database, and health status
- An append-only `state/events.jsonl`

The manifest—not a directory name, branch guess, or Docker search—is lifecycle authority. `destroy` validates containment, worktree ownership, Compose identity, and cleanliness before deleting anything. It never performs global Docker pruning.

## Agent-native operation

Create a new environment and enter it in one command:

```sh
stackstead launch feature-a -- codex
```

Run another command inside an existing environment by its full ID:

```sh
stackstead run <full-id> -- npm test
```

The child starts in the exact worktree with generated environment values plus pinned `STACKSTEAD_*` and `COMPOSE_PROJECT_NAME` identity. Stackstead preserves its exit status. See [Agent integration](docs/agent-integration.md).

When another manager owns worktree creation, bind it explicitly:

```sh
stackstead adopt feature-a --worktree /absolute/path/to/manager-worktree
stackstead up feature-a
stackstead run feature-a -- codex
stackstead destroy feature-a --yes  # runtime/state removed; external checkout preserved
```

The manifest records `source_ownership: external`, and its pointer must reciprocally identify that same full environment before any operation, so Stackstead cannot silently redirect or delete another manager-owned checkout. Ready-to-use Worktrunk, workmux, webmux, and generic hooks live in [`integrations/`](integrations); see [Manager integrations](docs/integrations.md).

## Inspect, recover, and clean up

```sh
stackstead ps
stackstead inspect feature-a
stackstead inspect feature-a --json
stackstead env feature-a
stackstead context feature-a --print
stackstead logs feature-a --tail 200
stackstead db status feature-a
stackstead open feature-a web
stackstead doctor
stackstead doctor --fail-on-error # CI: exit 1 only for error diagnostics
stackstead repair feature-a
stackstead stop feature-a
stackstead destroy feature-a --yes
```

`stop` preserves source and volumes. `repair` conservatively regenerates contract files and refreshes status. `destroy` refuses a dirty checkout before removing runtime data. Every JSON response is a command-owned version 1 object rather than a serialized manifest or internal result type.

## Proof, not promises

The [three-agent proof](examples/three-agent-demo/README.md) starts three real Nginx/Postgres stacks with the same migration ID but different branch payloads, proves six unique ports and three isolated databases, kills only one database, proves its peers survive, recovers the same state, retires a service, and proves exact manifest-led teardown removes its orphan without touching peers:

```sh
cargo build
STACKSTEAD_BIN="$PWD/target/debug/stackstead" \
  examples/three-agent-demo/demo.sh all /tmp/stackstead-three-agent-proof
```

This proof is a mandatory CI job through `scripts/docker-integration.sh`.

## Safety boundary

Stackstead isolates runtime identity and state, not hostile code. Processes still use the launching user's machine and Docker daemon permissions. Stackstead is not a security sandbox, secret manager, hosted-machine platform, browser controller, CI system, or production deployment tool.

Normal Compose-managed volumes are isolated by project identity. External or globally named volumes, host bind mounts, host networking, and services outside Compose can still share state; `doctor` reports common isolation breakers but does not rewrite arbitrary application topology.

## Documentation and gates

- [Quickstart](docs/quickstart.md)
- [Coding-agent setup v1](docs/agent-setup-v1.md)
- [Installation and release packaging](docs/install.md)
- [Configuration reference](docs/config.md)
- [Lifecycle and safe cleanup](docs/lifecycle.md)
- [Agent and manifest contract](docs/agent-contract.md)
- [Agent integration and run wrapper](docs/agent-integration.md)
- [Docker Compose requirements](docs/compose.md)
- [Postgres behavior](docs/database.md)
- [Existing-manager integrations](docs/integrations.md)
- [Rust architecture](docs/rust-architecture.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
scripts/test-install.sh
cargo build --locked --release
scripts/test-release-install.sh target/release/stackstead
scripts/test-delivery.sh
scripts/docker-integration.sh
```
