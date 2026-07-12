# Contributing to Stackstead

Stackstead favors small, explicit changes that preserve its fail-closed identity
and teardown contracts.

## Set up

Install a current Rust toolchain, Git, Docker, and the Docker Compose plugin.
Then run:

```sh
cargo build --locked
cargo test --locked
```

Use a focused branch and keep unrelated local changes out of the patch. Never
commit credentials, generated `.stackstead` state, or captured private
application data.

## Required checks

Run the checks relevant to the change; run the complete set before requesting a
release:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
scripts/test-install.sh
scripts/test-delivery.sh
cargo build --locked --release
scripts/test-release-install.sh target/release/stackstead
scripts/docker-integration.sh
```

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
