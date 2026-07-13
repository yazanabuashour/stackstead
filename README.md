# Stackstead: a real app stack for every coding agent

Run agents in parallel without sharing source, services, ports, or data.
Stackstead builds isolated environments on top of your existing Docker Compose
setup and works with Codex, Claude Code, Cursor, and worktree managers.

## Install

```sh
curl -fsSL https://github.com/yazanabuashour/stackstead/releases/latest/download/install.sh | sh
```

This installs the latest checksummed binary to `~/.local/bin`. Stackstead
supports macOS and glibc Linux and requires Git plus Docker with the Compose
plugin. See [Installation](docs/install.md) for platform details, pinned releases,
custom install paths, and building from source.

## Quick start

In a repository already configured for Stackstead:

```sh
stackstead launch feature-a -- codex
```

Stackstead creates a worktree and Compose project, allocates ports, starts the
services, and launches Codex inside that exact environment. Replace `codex` with
any agent or command.

To add Stackstead to a repository, ask your coding agent to follow the
[agent setup guide](docs/agent-setup-v1.md):

> Set up Stackstead in this repository. Follow the Stackstead agent setup v1
> guide, reuse the existing Compose setup, make the smallest changes needed,
> and show me the diff before committing.

Prefer to do it yourself? Follow the [manual quickstart](docs/quickstart.md).

## Why Stackstead

- **No collisions.** Every environment gets its own checkout, Compose project,
  ports, volumes, and database state.
- **No guessing.** A durable ID ties the checkout, runtime, generated context,
  and lifecycle state together.
- **Safe cleanup.** `stop` preserves source and data; `destroy` validates identity
  and checkout cleanliness before removing only the selected environment.
- **Use your existing tools.** Stackstead supplies the runtime beneath your agent,
  terminal, editor, dashboard, or worktree manager instead of replacing them.

## Everyday commands

```sh
stackstead ps
stackstead inspect <full-id>
stackstead logs <full-id> --tail 200
stackstead run <full-id> -- npm test
stackstead open <full-id> web
stackstead db status <full-id>
stackstead stop <full-id>
stackstead destroy <full-id> --yes
```

Use the full ID printed by `launch` in scripts and runtime-sensitive commands.
Inside an environment, use `$STACKSTEAD_ID` directly. See
[Agent integration](docs/agent-integration.md) for generated context and JSON
workflows.

## Existing worktree managers

When another tool owns the checkout, Stackstead can adopt it without taking
ownership of the source:

```sh
stackstead adopt feature-a --worktree /absolute/path/to/worktree
stackstead up feature-a
stackstead run feature-a -- codex
stackstead destroy feature-a --yes
```

The external checkout is preserved after teardown. Ready-to-use hooks for
Worktrunk, workmux, webmux, and generic launchers live in
[`integrations/`](integrations). See [Manager integrations](docs/integrations.md)
for the ownership and teardown contract.

## Proof, not promises

The [three-agent demo](examples/three-agent-demo/README.md) starts three real
Nginx/Postgres stacks, proves their ports and databases are isolated, recovers
one after failure, and tears it down without touching its peers. The same proof
runs in CI through `scripts/docker-integration.sh`.

## Safety boundary

Stackstead isolates development runtime identity and state; it is not a security
sandbox, secret manager, hosted environment, CI system, or production deployment
tool. Processes still run with the launching user's machine and Docker daemon
permissions.

Compose-managed resources are isolated by project identity. External or globally
named volumes, host bind mounts, host networking, and services outside Compose
can still share state. `stackstead doctor` reports common isolation breakers but
does not rewrite arbitrary application topology.

## Documentation

- Get started: [Quickstart](docs/quickstart.md) · [Agent setup](docs/agent-setup-v1.md)
  · [Installation](docs/install.md)
- Configure and operate: [Configuration](docs/config.md) ·
  [Lifecycle and cleanup](docs/lifecycle.md) · [Docker Compose](docs/compose.md) ·
  [Postgres](docs/database.md)
- Integrate: [Agent integration](docs/agent-integration.md) ·
  [Manager integrations](docs/integrations.md)
- Understand the contracts: [Agent and manifest contract](docs/agent-contract.md) ·
  [Rust architecture](docs/rust-architecture.md)
- Contribute: [Contributing](CONTRIBUTING.md) · [Security policy](SECURITY.md) ·
  [CI workflow](.github/workflows/ci.yml)
