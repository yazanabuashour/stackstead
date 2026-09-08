#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
require() { command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }
usage() {
  printf 'Usage: %s measure|profile <new-results-directory>\n' "$0"
  printf 'Set DOCKER_HOST to an explicitly authorized local unix:// socket. See docs/performance.md.\n'
}
if [[ ${1:-} == --help || ${1:-} == -h ]]; then usage; exit 0; fi
[[ $# == 2 ]] || { usage >&2; exit 2; }
mode="$1"
case "$mode" in measure|profile) ;; *) usage >&2; exit 2 ;; esac
[[ $(uname -s) == Linux ]] || die 'this workflow requires Linux'
for tool in bash env realpath mkdir git jq docker curl awk sed cp rm mv mktemp tr sort comm wc \
  sha256sum date uname grep head stat find; do require "$tool"; done
[[ -x /usr/bin/time && -x /usr/bin/true ]] || die 'GNU /usr/bin/time and /usr/bin/true are required'
/usr/bin/time --version | grep 'GNU' >/dev/null || die '/usr/bin/time must be GNU time'
if [[ $mode == measure ]]; then
  require hyperfine
  require strace
else
  require perf
  require heaptrack
  require readelf
fi
release_bin="$(realpath -e -- "${STACKSTEAD_BIN:-$repo_root/target/release/stackstead}")"
[[ -x $release_bin ]] || die "not executable: $release_bin"
binary="$release_bin"
if [[ $mode == profile ]]; then
  binary="$(realpath -e -- "${STACKSTEAD_PROFILE_BIN:-$repo_root/target/profiling/stackstead}")"
  [[ -x $binary && $binary != "$release_bin" ]] || die 'profiling requires a separate executable'
  sections="$(readelf --sections --wide "$binary")"
  grep '\.debug_info' <<<"$sections" >/dev/null || die 'profiling executable has no debug information'
  grep '\.symtab' <<<"$sections" >/dev/null || die 'profiling executable has no symbol table'
fi
case "${DOCKER_HOST:-}" in unix:///*) ;; *) die 'set DOCKER_HOST explicitly to an authorized local unix:///socket' ;; esac
results="$(realpath -m -- "$2")"
[[ ! -e $results && ! -L $results ]] || die "results directory already exists: $results"
case "$results/" in "$repo_root/"*) die 'place results outside the source checkout' ;; esac
fixture="$results/fixture"
source="$fixture/source"
ledger="$source/.demo-stacksteads.tsv"
# An allowlist keeps personal leases, Git hooks, credentials, and application variables out.
isolated=(env -i "PATH=$PATH" LANG=C LC_ALL=C "HOME=$fixture/home"
  "XDG_STATE_HOME=$fixture/state" "XDG_CONFIG_HOME=$fixture/config"
  "XDG_CACHE_HOME=$fixture/cache" "XDG_DATA_HOME=$fixture/data"
  "XDG_RUNTIME_DIR=$fixture/runtime" "TMPDIR=$fixture/tmp"
  "DOCKER_CONFIG=$fixture/docker" "DOCKER_HOST=$DOCKER_HOST"
  GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
  "GIT_TEMPLATE_DIR=$fixture/git-template" GIT_CONFIG_COUNT=1
  GIT_CONFIG_KEY_0=core.hooksPath "GIT_CONFIG_VALUE_0=$fixture/hooks"
  "STACKSTEAD_BIN=$release_bin")
# prepare copies the demo directory. Refuse untracked/ignored additions and generated secrets first.
untracked="$("${isolated[@]}" git -C "$repo_root" ls-files --others -- examples/three-agent-demo)"
[[ -z $untracked ]] || die 'demo source contains untracked or ignored files; use a clean fixture source'
unsafe="$(find "$repo_root/examples/three-agent-demo" \
  \( -name '.env' -o -name '.env.*' -o -name '.stackstead' -o -type l \) -print)"
[[ -z $unsafe ]] || die 'demo source contains environment files, generated state, or symlinks'
# These read-only probes precede fixture creation. User-local Docker plugins are not inherited.
"${isolated[@]}" docker compose version >/dev/null || die 'system-installed Docker Compose plugin required'
"${isolated[@]}" docker info --format '{{.ServerVersion}}' >/dev/null || die 'authorized Docker daemon unavailable'

umask 077
mkdir -- "$results"
mkdir -p -- "$fixture"/{home,state,config,cache,data,runtime,tmp,docker,hooks,git-template/info}
# The private parent protects receipts; container users must be able to read checked-out app files.
umask 022
printf 'Results: %s\n' "$results"
record() {
  {
    printf 'cd %q && ' "$PWD"
    printf '%q ' "$@"
    printf '\n'
  } >>"$results/commands.txt"
}
run() { record "${isolated[@]}" "$@"; "${isolated[@]}" "$@"; }
finish() {
  status=$?
  trap - EXIT
  set +e
  if [[ -f $ledger ]]; then
    cp -- "$ledger" "$results/cleanup-before.tsv" || status=1
    if [[ $status != 0 ]]; then
      while IFS=$'\t' read -r _ failed_id _; do
        (cd "$source" && run "$release_bin" logs "$failed_id") ||
          printf 'could not collect runtime logs for %s\n' "$failed_id"
      done <"$ledger" >"$results/failure-runtime.log" 2>&1
    fi
  fi
  if [[ -x $source/demo.sh ]]; then
    if run /usr/bin/time -v -o "$results/cleanup.time" "$source/demo.sh" cleanup \
      >"$results/cleanup.log" 2>&1; then
      printf 'succeeded\n' >"$results/cleanup.status"
    else
      cleanup_status=$?
      printf 'failed exit=%s\n' "$cleanup_status" >"$results/cleanup.status"
      [[ ! -f $ledger ]] || cp -- "$ledger" "$results/cleanup-remaining.tsv"
      printf 'Cleanup failed. Retained exact ledger: %s\nRetry command: %s\n' \
        "$ledger" "$results/cleanup-command.txt" >&2
      status=1
    fi
  else
    printf 'prepare did not install demo.sh; inspect retained fixture\n' >"$results/cleanup.status"
  fi
  if [[ -f $results/binaries.sha256 ]]; then
    sha256sum --check "$results/binaries.sha256" >"$results/binaries-after.txt" 2>&1 || status=1
  fi
  printf '%s\n' "$status" >"$results/exit-status.txt"
  printf 'Retained receipts and fixture at %s. No recursive fixture deletion was attempted.\n' "$results" >&2
  exit "$status"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
printf '%q ' "${isolated[@]}" "$source/demo.sh" cleanup >"$results/cleanup-command.txt"
printf '\n' >>"$results/cleanup-command.txt"
sha256sum "$release_bin" "$binary" >"$results/binaries.sha256"
{
  date --utc --iso-8601=seconds
  uname -a
  printf 'mode=%s\nrelease=%s\nmeasured=%s\n' "$mode" "$release_bin" "$binary"
  printf 'Source checkout revision, not proof of binary provenance:\n'
  run git -C "$repo_root" rev-parse HEAD
  printf 'Tracked diff against HEAD, including staged changes, SHA-256:\n'
  run git -C "$repo_root" diff --no-ext-diff --no-textconv --binary HEAD | sha256sum
  printf 'Checkout status, including untracked paths:\n'
  run git -C "$repo_root" status --short
  printf 'Input hashes:\n'
  sha256sum "$repo_root"/{Cargo.toml,Cargo.lock,rust-toolchain.toml,scripts/benchmark.sh} \
    "$repo_root/examples/three-agent-demo/"{demo.sh,stackstead.yaml,docker-compose.yml}
  printf 'CPU, allowed CPUs, filesystem, and host load before setup:\n'
  awk '/^(model name|Hardware)/ { print; exit }' /proc/cpuinfo
  grep '^Cpus_allowed_list:' /proc/self/status
  stat -f -c 'filesystem=%T' "$results"
  head -n 1 /proc/loadavg
} >"$results/identity.txt"
{
  run "$release_bin" --version
  [[ $mode != profile ]] || run "$binary" --version
  run bash --version
  run git --version
  run jq --version
  run docker --version
  run docker compose version
  run docker info --format 'server={{.ServerVersion}} storage={{.Driver}} cpus={{.NCPU}} memory={{.MemTotal}}'
  run curl --version
  run /usr/bin/time --version
  if [[ $mode == measure ]]; then run hyperfine --version; run strace --version;
  else run perf --version; run heaptrack --version; run readelf --version; fi
} >"$results/tool-versions.txt" 2>&1
printf '%s\n' \
  'No build runs here. Expected release: cargo build --locked --release, stock settings.' \
  'Expected profiling: cargo build --locked --profile profiling, release inheritance, debug=true, strip=false.' \
  'Binary overrides and prior build flags are not inferable. Attach your exact build commands and logs.' \
  'Three live demo environments; queries run from alpha worktree, stdout discarded.' \
  'Hyperfine warmup=3, samples=21, reused from audit 68a4c0d; first-pass counts, not limits.' \
  'No cache eviction, image pull, CPU pinning, frequency control, or background-load control.' \
  'Create/up timings include image pulls if absent, database initialization, readiness, and Git commits.' \
  'Repeated queries follow create and verify. Images and filesystem caches are then warm.' \
  'GNU time RSS includes waited-descendant high-water marks, not aggregate tree/container memory.' \
  >"$results/method.txt"
# Inspect only public image identity fields, never container configuration or environment.
while read -r image; do
  printf 'image=%s\n' "$image"
  if ! run docker image inspect --format '{{.Id}} {{json .RepoDigests}}' "$image"; then
    printf 'not locally inspectable before setup\n'
  fi
done < <(awk '$1 == "image:" { print $2 }' "$repo_root/examples/three-agent-demo/docker-compose.yml") \
  >"$results/images-before.txt" 2>&1
run /usr/bin/time -v -o "$results/prepare.time" \
  "$repo_root/examples/three-agent-demo/demo.sh" prepare "$source" >"$results/prepare.log" 2>&1
run /usr/bin/time -v -o "$results/create.time" "$source/demo.sh" create >"$results/create.log" 2>&1
run /usr/bin/time -v -o "$results/verify-before.time" "$source/demo.sh" verify >"$results/verify-before.log" 2>&1
cp -- "$ledger" "$results/identities.tsv"
[[ $(wc -l <"$ledger") == 3 ]] || die 'expected exactly three demo identities'
while IFS=$'\t' read -r agent id worktree _ project; do
  printf '%s %s %s\n' "$agent" "$id" "$project" >>"$results/fixture-commits.txt"
  run git -C "$worktree" rev-parse HEAD >>"$results/fixture-commits.txt"
  inspection="$(cd "$worktree" && run "$release_bin" --json inspect "$id")"
  jq -e --arg id "$id" '.kind == "StacksteadInspection" and .version == "4" and
    .stackstead.stackstead_id == $id and .live.runtime.running and .live.readiness.status == "ready"' <<<"$inspection" >/dev/null
  while read -r container; do
    run docker inspect --format '{{.Id}} {{.Image}}' "$container" >>"$results/container-images.txt"
  done < <(jq -r '.live.services[].id' <<<"$inspection")
done <"$ledger"
IFS=$'\t' read -r agent id worktree _ _ <"$ledger"
[[ $agent == alpha && $worktree == "$fixture/"* ]] || die 'alpha worktree escaped the fixture parent'
cd -- "$worktree"
printf 'workload\tsample\telapsed_s\tuser_s\tsystem_s\tmaxrss_KiB\texit\n' >"$results/rss.tsv"
head -n 1 /proc/loadavg >"$results/load-before-samples.txt"
for name in true ps inspect-json inspect-human current run-true; do
  case "$name" in
    true) command=(/usr/bin/true) ;;
    ps) command=("$binary" --json ps) ;;
    inspect-json) command=("$binary" --json inspect "$id") ;;
    inspect-human) command=("$binary" inspect "$id") ;;
    current) command=("$binary" --json current) ;;
    run-true) command=("$binary" run "$id" -- /usr/bin/true) ;;
  esac
  if [[ $mode == measure ]]; then
    # @sh supplies portable quoting for hyperfine's shell-words parser. No timed shell wrapper.
    quoted="$(jq -nr --args '$ARGS.positional | @sh' -- "${command[@]}")"
    run hyperfine --shell=none --warmup 3 --runs 21 \
      --export-json "$results/$name.hyperfine.json" "$quoted" >"$results/$name.hyperfine.log" 2>&1
    for ((sample=1; sample<=21; sample++)); do
      run /usr/bin/time -a -o "$results/rss.tsv" \
        -f "$name\t$sample\t%e\t%U\t%S\t%M\t%x" "${command[@]}" >/dev/null
    done
    run strace -f -qq -e trace=%process,%file,poll,ppoll,nanosleep,clock_nanosleep -e abbrev=all \
      -o "$results/$name.strace" -- "${command[@]}" >/dev/null 2>"$results/$name.strace.stderr"
  elif [[ $name != true ]]; then
    # The loop gives short commands more sampling opportunities; attribute shell/client costs separately.
    run perf record -e cpu-clock:u --call-graph dwarf -o "$results/$name.perf.data" -- \
      bash -c 'for ((i=0; i<21; i++)); do "$@" >/dev/null || exit; done' benchmark "${command[@]}" \
      >"$results/$name.perf.stdout" 2>"$results/$name.perf.stderr"
    run perf report --stdio -i "$results/$name.perf.data" \
      >"$results/$name.perf.txt" 2>"$results/$name.perf-report.stderr"
    run heaptrack --output "$results/$name.heaptrack" "${command[@]}" \
      >"$results/$name.heaptrack.stdout" 2>"$results/$name.heaptrack.stderr"
  fi
done
head -n 1 /proc/loadavg >"$results/load-after-samples.txt"
# A second calibration catches a changed launcher RSS floor; keep raw zero/rounded values.
if [[ $mode == measure ]]; then
  for ((sample=1; sample<=21; sample++)); do
    run /usr/bin/time -a -o "$results/rss.tsv" \
      -f "true-after\t$sample\t%e\t%U\t%S\t%M\t%x" /usr/bin/true >/dev/null
  done
fi
cd -- "$source"
run /usr/bin/time -v -o "$results/verify-after.time" "$source/demo.sh" verify >"$results/verify-after.log" 2>&1
