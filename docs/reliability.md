# Reliability evidence

Stackstead is pre-1.0 software. Use executable checks and receipts tied to the
revision under evaluation, not a past release's results, to assess a change.

## Reproduce the checks

Run [`scripts/ci.sh`](../scripts/ci.sh) for the full local Linux sequence, or its
`rust`, `docker`, and `macos` modes for focused jobs. Follow the
[contributor prerequisites](../CONTRIBUTING.md#required-checks), especially the
disposable Docker requirement. Unit tests and command mocks do not establish
live startup, isolation, or teardown behavior.

The [three-agent demo](../examples/three-agent-demo/README.md) exercises separate
Nginx/Postgres environments, peer preservation during failure and recovery, and
exact-ID teardown. Save gate logs with the source revision and execution
environment. A documented check is not a passing result for the current change.

For query timing methodology and saved live samples, see
[performance measurement](performance.md) and its
[baseline receipt](../receipts/performance-live-baseline.json). Those samples
cover the named workloads and binary only, not concurrent lifecycle performance.
Unavailable-daemon timings do not establish live-runtime latency.

## What this evidence does not establish

- Market demand or team-wide adoption.
- Every valid Docker Compose shape or every application framework.
- Windows support, musl Linux binaries, or remote orchestration.
- Host isolation from code that can access the invoking user's Docker daemon.
- Detached-session cleanup on macOS beyond the documented process-group
  best-effort boundary.

Linux's detached-descendant cleanup and macOS's process-group cleanup have
different boundaries; see [agent integration](agent-integration.md). Passing a
Linux gate does not replace macOS validation or evidence from other applications.
