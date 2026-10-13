# Stackstead documentation

Start with the guide that matches what you are trying to do.

## Get started

- [Why Stackstead?](why-stackstead.md) explains the problem Stackstead solves,
  its operating model, and when it is not the right tool.
- [Installation](install.md) covers supported platforms, checksummed releases,
  upgrades, and source builds.
- [Quickstart](quickstart.md) configures and launches a first environment.
- [Agent setup](agent-setup.md) is the copyable setup contract for coding agents.

## Configure and operate

- [Configuration](config.md) documents `stackstead.yaml`.
- [Docker Compose](compose.md) covers discovery, port rewrites, service commands,
  project identity, and volume isolation.
- [Lifecycle and cleanup](lifecycle.md) covers create, adopt, up, stop, repair,
  destroy, locking, and teardown recovery.
- [Database support](database.md) covers the Postgres lifecycle and boundary.

## Integrate

- [Agent integration](agent-integration.md) covers host/service execution,
  generated context, and injected environment.
- [Manager integrations](integrations.md) covers Worktrunk, workmux, webmux, and
  generic launchers.
- [Agent contract](agent-contract.md) is the CLI JSON and persisted-identity
  reference.

## Upgrade

- [Upgrade from 0.1.3 to 0.1.4](upgrade-0.1.4.md) covers the inspection transport,
  hook rollout, and exact-target recovery for an already failed teardown.
- [Upgrade from 0.1.4 to 0.1.5](upgrade-0.1.5.md) covers the current-worktree
  identity contract and matching manager-hook rollout.
- [Stackstead 0.1.4 release notes](releases/v0.1.4.md) explain that release's
  reliability changes. Use the current contract reference for current JSON.

## Trust and participate

- [Reliability evidence](reliability.md) explains reproducible checks and evidence
  limits.
- [Performance measurement](performance.md) documents query timing and profiling
  receipts.
- [Early adopter program](early-adopters.md) explains who the current release is
  for and how to participate.
- [Contributing](../CONTRIBUTING.md) covers the code map, checks, and release
  packaging; [Rust architecture](rust-architecture.md) explains internal invariants.
- [Security policy](../SECURITY.md) covers private vulnerability reporting.
