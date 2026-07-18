# Stackstead documentation

Start with the guide that matches what you are trying to do.

## Get started

- [Why Stackstead?](why-stackstead.md) explains the problem Stackstead solves,
  its operating model, and when it is not the right tool.
- [Installation](install.md) covers supported platforms, checksummed releases,
  upgrades, and source builds.
- [Upgrade from 0.1.3 to 0.1.4](upgrade-0.1.4.md) covers the inspection transport,
  hook rollout, and exact-target recovery for an already failed teardown.
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

- [Agent integration](agent-integration.md) covers generated context, JSON, and
  host or service command execution.
- [Manager integrations](integrations.md) covers Worktrunk, workmux, webmux, and
  generic launchers.
- [Agent and manifest contract](agent-contract.md) documents generated state and
  versioned transport boundaries.

## Trust and participate

- [Reliability evidence](reliability.md) summarizes dogfood and release-gate
  results, including failures and limitations.
- [Early adopter program](early-adopters.md) explains who the current release is
  for and how to participate.
- [Rust architecture](rust-architecture.md) documents the internal boundaries and
  deliberate scope budget.
- [Contributing](../CONTRIBUTING.md) and [Security policy](../SECURITY.md) cover
  changes and private vulnerability reporting.
- [Stackstead 0.1.4 release notes](releases/v0.1.4.md) describe the current
  reliability release and upgrade considerations.
