# Contributing to Stackstead

Stackstead favors small, explicit changes that preserve its fail-closed identity
and teardown contracts.

## Set up

Use the Rust toolchain pinned in [`rust-toolchain.toml`](rust-toolchain.toml).
Install Git; live runtime checks also require Docker and the Compose plugin.
Then run:

```sh
cargo build --locked
cargo test --locked
```

Use a focused branch and keep unrelated local changes out of the patch. Never
commit credentials, generated `.stackstead` state, or captured private
application data.

## Required checks

Run [`scripts/ci.sh`](scripts/ci.sh) for the full local Linux sequence. Use
`scripts/ci.sh rust`, `scripts/ci.sh docker`, or `scripts/ci.sh macos` for the
corresponding focused job. Run relevant checks before submitting a change and
the full sequence before requesting a release. Record any unavailable or blocked
checks.

The full Linux sequence and `docker` mode create and destroy live fixtures.
Use an explicitly authorized disposable Docker daemon, not a runtime supporting
daily work. Keep the fixture source and sibling state in a fresh parent; do not
reuse application paths. The Docker integration isolates its port-lease registry
and retains failed fixtures for exact-ID cleanup. Follow its printed retry
command with the same Docker connection and state paths. Never use broad Docker
cleanup. See the [three-agent demo](examples/three-agent-demo/README.md).

`scripts/ci.sh docker` builds the debug binary first. When invoking
`scripts/docker-integration.sh` directly, either build it first or set
`STACKSTEAD_BIN` to an explicit executable.

Changes to manifests, pointers, generated context, CLI/JSON output, config,
ports, Compose identity, locks, or teardown need regression coverage for both
the successful path and ambiguous/tampered input. Docker lifecycle changes need
live integration evidence; unit-only mocks are not enough.

## Pull requests

Explain the user-visible outcome, safety implications, tests run, and any
compatibility or migration impact. Prefer the smallest solution that meets the
current use case. Do not push releases, create tags, or modify external systems
as part of a contribution.

Report suspected vulnerabilities through [SECURITY.md](SECURITY.md), not a
public issue.
