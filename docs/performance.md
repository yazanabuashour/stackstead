# Measure CLI performance

Use [`scripts/benchmark.sh`](../scripts/benchmark.sh) to measure the existing
[three-agent demo](../examples/three-agent-demo/README.md) against an explicitly
authorized local Docker daemon. It creates a fresh copy, starts three isolated
environments, verifies them, measures queries, verifies again, and attempts
identity-bound cleanup even when a command fails. It never builds Stackstead.

Do not run this against an existing application's checkout or state. Use a
quiescent disposable daemon where possible. A private lease registry cannot
coordinate with other Stackstead processes using a different registry on the
same daemon. Do not run competing fixtures, builds, tests, or profilers during
sampling. A successful run does not prove concurrent lifecycle performance.

## Prepare tools and binaries

Linux prerequisites are Bash, GNU coreutils and findutils, GNU `/usr/bin/time`,
Git, jq, curl, Docker Engine, and a system-installed Docker Compose plugin. Measurement
also requires hyperfine and strace. Profiling requires perf, Heaptrack, and
binutils `readelf`. Install these with the host's package manager first.
User-local Docker plugins and credentials are deliberately not inherited.
`cargo-audit` belongs to the separate dependency security gate, not these timings.

Build both executables before sampling, using the repository toolchain. These
commands assume no Cargo profile overrides, `RUSTFLAGS`, or
`CARGO_ENCODED_RUSTFLAGS`; check local Cargo configuration too. Keep the exact
build commands and logs beside the results. Finish all builds before running
either mode.

```sh
cargo build --locked --release
cargo build --locked --profile profiling
```

The stock release uses stripping and thin link-time optimization. The
`profiling` profile inherits release optimization, enables debug information,
and disables stripping. It is for attribution, not the release timing baseline.
Do not substitute a debug build. The script checks the profiling executable for
symbols and debug information, but cannot prove its optimization flags or source
revision from the executable alone.

## Collect receipts

Choose a results path outside the checkout whose parent exists. The final
results directory must not exist. Set `DOCKER_HOST` to the actual authorized
Unix socket, not an inferred Docker context. The example assumes you have
explicitly authorized `/var/run/docker.sock`; replace it for a disposable daemon.

```sh
DOCKER_HOST=unix:///var/run/docker.sock \
  scripts/benchmark.sh measure /tmp/stackstead-before

DOCKER_HOST=unix:///var/run/docker.sock \
  scripts/benchmark.sh profile /tmp/stackstead-profile
```

`STACKSTEAD_BIN` overrides `target/release/stackstead`.
`STACKSTEAD_PROFILE_BIN` overrides `target/profiling/stackstead` in profile mode.
Both accept executable paths; the script resolves them to absolute paths before
changing HOME or working directory. Profile mode still uses the release binary
for fixture setup, verification, and cleanup. Use binaries from the same source
and do not rebuild or replace either during a run. Hash checks at exit detect
replacement. The script does not copy the binaries, so preserve them separately
if you need later symbolization.

Both modes use the demo's `prepare`, `create`, `verify`, and `cleanup` commands.
The fixture has alpha, beta, and gamma worktrees with different committed
Postgres initialization data, plus web and completed setup services. All query
commands run from alpha's recorded worktree while all three environments exist:

- `stackstead --json ps`
- `stackstead --json inspect <alpha-full-id>`
- `stackstead inspect <alpha-full-id>`
- `stackstead --json current`
- `stackstead run <alpha-full-id> -- /usr/bin/true`

The script captures full identities from the demo ledger, not guessed slugs or
Compose names. Human and JSON inspection are separate workloads so duplicate
presentation work remains visible. `current` resolves the real generated
worktree pointer. `run` includes supervisor and lease work, not just `true`.

### Release measurements

`measure` runs hyperfine with no shell wrapper, three warmups, and 21 samples
per command. It also measures direct `/usr/bin/true`. JSON files contain every
raw timing in `results[].times`, in seconds. Workloads run in the listed script
order, not randomized round-robin order. Query stdout is discarded.

The [live baseline receipt](../receipts/performance-live-baseline.json) contains
raw samples, binary and fixture hashes, tool versions, resource samples, and
cleanup results. The sample counts are a first pass, not operating limits or a
speed budget. The earlier audit used synthetic environments and an unavailable
Docker daemon. Its `ps` and `inspect` timings are not comparable to this live demo.

GNU time records 21 separate resource samples per workload in `rss.tsv`, plus
direct `true` calibrations before and after. Treat an indistinguishable launcher
floor, zero RSS, or rounded short CPU times as unresolved measurement, not zero
cost. Do not subtract the calibration and call the remainder heap usage. Linux
maximum resident set size is a high-water mark including waited descendants,
not their sum. Docker and Compose clients can dominate; daemon and container
memory are outside this measurement. GNU time wall times do not replace
hyperfine's finer timing samples.

Each workload then gets a separate stock-release strace using only
`%process,%file,poll,ppoll,nanosleep,clock_nanosleep`, following launched
descendants. The wait calls distinguish event waits from timed polling.
Environment abbreviation stays enabled. There are no read/write buffers, environment dumps, network payloads,
or hand-written tracing collectors. The trace explains subprocess and file
activity; its durations are not uninstrumented performance results.

### Optimized profiling

`profile` runs `perf record -e cpu-clock:u --call-graph dwarf` around 21
invocations of each query and saves a text report. The loop increases sampling
opportunities for short commands. Distinguish Stackstead, loop-shell, Git,
Docker, and Compose frames; do not attribute the whole process tree to
Stackstead. The raw `.perf.data`, stderr, and `.perf.txt` remain available.
Check sample counts and resolved Stackstead frames before claiming a hotspot.
A short command can still have insufficient samples.

Each query also runs once under Heaptrack, retaining its raw allocation trace
and logs. Use `heaptrack_gui`, or analyze it without a desktop:

```sh
heaptrack_print --file /tmp/stackstead-profile/current.heaptrack.zst | c++filt --format=rust
```

The optional `heaptrack_print` analyzer resolves source lines; GNU binutils
`c++filt` demangles Rust names. Check which processes the trace covers.
Allocations, retained heap, RSS, and container memory answer different questions.

Perf availability and userspace event permissions vary by kernel and host
policy. A permitted event is not proof of useful call stacks. The script fails
and attempts cleanup if profiling fails; it does not change sysctls, escalate
privileges, or silently replace the profiler. Perf stack data and heap traces
can contain process data. Profile only this disposable fixture with its public
demo credentials, never an application carrying secrets.

## Read setup and cache receipts

`prepare.time`, `create.time`, `verify-*.time`, and `cleanup.time` are separate
GNU time phase receipts. `create.log` retains the existing human `up` phase
timings. These are single setup observations, not repeated lifecycle samples.
Create includes Git commits, any missing image downloads, database initialization,
and readiness. Do not add those times to repeated query latency.

`images-before.txt` records whether each fixture tag was locally inspectable;
`container-images.txt` records actual container and immutable image IDs after
startup, including the completed setup service. Public image tags can move.
Compare image IDs, not tags alone.

Queries follow startup and verification, so filesystem and runtime caches are
warm. The script never evicts host caches, deletes images, or restarts Docker.
A fresh fixture is not a cold filesystem. Missing images distinguish a download
case, not a disk-cold query. Measure cold behavior separately only with explicit
authorization on a disposable host, and record the cache preparation commands.

`commands.txt` records exact arguments, the isolated environment allowlist, and
working directories. `identity.txt` records checkout revision, tracked-diff hash,
status, input hashes, CPU, affinity, filesystem type, and initial load.
`binaries.sha256` is the authoritative executable identity; checkout metadata is
not proof of how an override binary was built. `tool-versions.txt`, `method.txt`,
and load snapshots record the remaining execution context. No environment dump
or source diff contents are saved.

## Recover cleanup safely

The results directory owns a fresh `fixture/` parent containing source, sibling
project state, HOME, XDG directories, temporary files, Docker configuration, and
an empty Git template with its required `info` directory. The results parent
has mode `0700`; app files use ordinary permissions so container users can read
bind-mounted files. Git system/global configuration and hooks are disabled for fixture commands. Preparation refuses
untracked or ignored demo files, generated environment files, and symlinks before
copying. No personal lease registry or application environment is inherited.

The exit handler always attempts the copied demo's cleanup once preparation has
installed it. Cleanup uses the exact full IDs in
`fixture/source/.demo-stacksteads.tsv` through Stackstead's normal ownership and
dirty-state checks. It never calls prune or guesses a destructive target.
`cleanup-before.tsv` preserves the attempted identities; on failure,
`cleanup-remaining.tsv`, `cleanup.log`, and the live ledger preserve recovery
context. Failed runs also attempt exact-ID runtime logs before cleanup, saved in
`failure-runtime.log`. `cleanup.status` records the cleanup result and `exit-status.txt`
records the workflow result. SIGKILL or host failure cannot run an exit handler.

On failure, retain the entire fixture. Inspect the logs and exact ledger, then
review and run the single command in `cleanup-command.txt`. It preserves the
original absolute release binary, Docker endpoint, HOME, and XDG paths. Check
its binary against `binaries.sha256` first. The retry does not update the earlier
workflow status receipts. Do not remove the ledger, repair generated files by
hand, or recursively delete uncertain runtime state to make cleanup pass.

The script leaves the parent directory even after successful cleanup. Archive
receipts, not the entire `fixture/` tree: a failed fixture can still contain
`.stackstead/.env`. Never print, copy, or retain generated environment contents
as performance evidence. Remove a retained parent only after identity-bound
cleanup and confirmation that no unresolved resources remain.

## Compare a change

Collect a `measure` run before and after with stock-release binaries, the same
host, image IDs, fixture, tool versions, and quiet-host conditions. Keep both
raw result directories and build receipts. Compare distributions and outliers,
not the fastest sample. A median or small-sample p95 shift within ordinary
variance is not an optimization claim. Repeat runs in alternating revision
order when drift matters; record any changed setup or cache state.

Use separate profile receipts to explain a repeatable difference, then rerun
uninstrumented release measurements. No receipt, no number. Do not introduce a
speed budget without healthy workload samples; any future tripwire belongs
beyond measured healthy behavior, with its receipt attached.

The [audit follow-up receipt](../receipts/performance-audit-followup.json) keeps
live samples, binary hashes and source identities for the readiness and supervisor
changes. Readiness adds observation work, so its inspection timings are not an
apples-to-apples speed comparison with the original baseline. Compare the host
supervisor through `run -- true`; its separate trace records event waits without
reading command buffers or environment contents.
