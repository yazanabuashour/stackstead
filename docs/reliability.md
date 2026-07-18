# Reliability evidence

Stackstead is pre-1.0 software. Its reliability claims are based on executable
tests and supervised dogfood, not on a claim that every Compose topology is
already supported.

## July 2026 dogfood

Stackstead 0.1.3 was exercised in ten issue-shaped coding sessions across two
applications, including an unfamiliar Plane fixture. Nine sessions reached
their runtime-dependent acceptance and eight did so without Stackstead-specific
operator correction.

The program validated:

- distinct source, Compose identity, ports, URLs, and persistent state across
  two independent three-agent waves;
- no task-data crossover or peer damage;
- generated context that agents could use without guessed runtime details;
- stop and resume with the same identity, ports, source, and database data; and
- documented recovery of a stopped full runtime while a peer stayed healthy.

It also found four product-level reliability problems. Parallel lifecycle entry
failed immediately on shared locks, interrupted `stackstead run` children could
retain a run lease, inspection did not clearly resolve recorded/live divergence,
and a failed teardown could strand source and one owned volume after encountering
container-created root-owned files. The last teardown failures blocked a broad
0.1.3 reliability recommendation.

## 0.1.4 focused rerun

Version 0.1.4 keeps a narrow fix for each reproduced failure:

| Challenge | Required behavior | Result |
| --- | --- | --- |
| Parallel entry | Contending creates wait within a bounded lock budget and retain distinct identities. | Pass; contenders completed in 3.3 and 6.1 seconds. |
| Diagnosis | Inspection distinguishes recorded, live, and effective status deterministically. | Pass; the stopped target failed effectively while six peer services stayed running. |
| Interrupted run | Direct and detached descendants disappear before lifecycle reacquires the run lease. | Pass on Linux; lifecycle reacquired in 1.4 seconds. |
| Teardown retry | An exact-ID retry resumes the failed phase, removes root-owned output and owned resources, does not rerun the hook, and preserves a healthy peer. | Pass; a third destroy was a no-mutation not-found result. |

The four challenges ran against the 0.1.4 release binary in a fresh, remote-free
Plane fixture. The release candidate also passes formatting, Clippy with
warnings denied, unit and CLI acceptance tests, installer packaging, clean
release installation, delivery tests, macOS CI, and the live three-environment
Docker proof.

## What this evidence does not establish

- Market demand or team-wide adoption.
- Every valid Docker Compose shape or every application framework.
- Windows support, musl Linux binaries, or remote orchestration.
- Host isolation from code that can access the invoking user's Docker daemon.
- Detached-session cleanup on macOS beyond the documented process-group
  best-effort boundary.

The initial dogfood used one operator, one host, two repositories, and at most
three agents at once. The focused detached-descendant gate is authoritative on
Linux; macOS is covered by compilation and CI behavior. External early-adopter
results are the next evidence needed.

To reproduce the public runtime proof, run the
[three-agent demo](../examples/three-agent-demo/README.md). To exercise the full
repository gate, run `scripts/ci.sh` from a reviewed checkout.
