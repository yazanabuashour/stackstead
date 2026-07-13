# Stackstead

**Run parallel coding agents against your real app—without sharing ports, services, or databases.**

- **Your real stack:** reuse the existing Compose topology instead of maintaining
  a second agent-specific environment.
- **One identity:** source, services, ports, data, URLs, and lifecycle belong to
  one durable environment ID.
- **Safe teardown:** clean up one agent’s runtime without touching another
  agent’s work or state.

Stackstead works with Codex, Claude Code, Cursor, other coding agents, and
worktree managers.

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
stackstead launch feature-a -- claude
```

Stackstead creates a worktree and Compose project, allocates ports, starts the
services, and launches Claude inside that exact environment. Replace `claude` with
any agent or command.

To add Stackstead to a repository, ask your coding agent to follow the
[agent setup guide](docs/agent-setup.md):

> Set up Stackstead in this repository. Follow the Stackstead agent setup guide,
> reuse the existing Compose setup, make the smallest changes needed, and show
> me the diff before committing.

Prefer to do it yourself? Follow the [manual quickstart](docs/quickstart.md).

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
stackstead up <full-id>
stackstead run <full-id> -- claude
stackstead destroy <full-id> --yes
```

The external checkout is preserved after teardown. Ready-to-use hooks for
Worktrunk, workmux, webmux, and generic launchers live in
[`integrations/`](integrations). See [Manager integrations](docs/integrations.md)
for the ownership and teardown contract.

## Verified harmony

The [three-agent demo](examples/three-agent-demo/README.md) starts three real
Nginx/Postgres stacks, proves their ports and databases are isolated, recovers
one after failure, and tears it down without touching its peers. The same proof
runs in CI through `scripts/docker-integration.sh`.

## Documentation

- Get started: [Quickstart](docs/quickstart.md) · [Agent setup](docs/agent-setup.md)
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
