#!/usr/bin/env bash
set -Eeuo pipefail

example_root="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
state_suffix="$(printf '%s' "$(basename "$example_root")" | tr -c 'A-Za-z0-9._-' '_')"
state_root="$example_root/../.stackstead-state-$state_suffix"
ledger="$example_root/.demo-stacksteads.tsv"
stackstead_bin="${STACKSTEAD_BIN:-stackstead}"

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

require_runtime() {
  require git
  require jq
  require docker
  command -v "$stackstead_bin" >/dev/null 2>&1 || die "Stackstead executable not found: $stackstead_bin"
  docker compose version >/dev/null 2>&1 || die "Docker Compose plugin is required"
}

prepare() {
  [ "$#" -eq 1 ] || die "usage: $0 prepare <empty-destination>"
  destination="$1"
  (umask 077 && mkdir "$destination") 2>/dev/null ||
    die "destination already exists or cannot be created: $destination"
  if [ -n "${STACKSTEAD_PREPARE_OWNER_TOKEN:-}" ]; then
    printf '%s\n' "$STACKSTEAD_PREPARE_OWNER_TOKEN" >"$destination/.stackstead-docker-test-owner"
  fi
  destination="$(CDPATH= cd -- "$destination" && pwd -P)"
  case "$destination/" in
    "$example_root/"*) die "destination must be outside the example source" ;;
  esac
  cp -R "$example_root/." "$destination/"
  destination_suffix="$(printf '%s' "$(basename "$destination")" | tr -c 'A-Za-z0-9._-' '_')"
  sed -i "s|\.stackstead-state-three-agent-demo|.stackstead-state-$destination_suffix|" \
    "$destination/stackstead.yaml"
  rm -f "$destination/.demo-stacksteads.tsv"
  git -C "$destination" init -b main >/dev/null
  [ ! -f "$destination/.stackstead-docker-test-owner" ] ||
    printf '/.stackstead-docker-test-owner\n' >>"$destination/.git/info/exclude"
  git -C "$destination" add .
  git -C "$destination" \
    -c user.name='Stackstead Demo' \
    -c user.email='stackstead-demo.invalid' \
    commit -m 'Initialize three-agent isolation proof' >/dev/null
  printf 'Prepared isolated demo repository: %s\n' "$destination"
}

write_branch_contract() {
  worktree="$1"
  agent="$2"
  payload="$3"
  cat >"$worktree/db/init/001_branch_contract.sql" <<SQL
CREATE TABLE schema_migrations (
  migration_id text PRIMARY KEY,
  agent text NOT NULL,
  payload text NOT NULL
);

CREATE TABLE seeded_accounts (
  account_id integer PRIMARY KEY,
  owner text NOT NULL
);

INSERT INTO schema_migrations VALUES
  ('202607090001', '$agent', '$payload');
INSERT INTO seeded_accounts VALUES (1, '$agent');
SQL
  git -C "$worktree" add db/init/001_branch_contract.sql
  git -C "$worktree" \
    -c user.name='Stackstead Demo Agent' \
    -c user.email='stackstead-agent.invalid' \
    commit -m "Agent $agent changes migration 202607090001" >/dev/null
}

payload_for_agent() {
  case "$1" in
    alpha) printf '%s\n' 'adds invoice_state=queued' ;;
    beta) printf '%s\n' 'adds invoice_state=processing' ;;
    gamma) printf '%s\n' 'adds invoice_state=settled' ;;
    *) die "unexpected demo agent: $1" ;;
  esac
}

register_cell() {
  name="$1"
  [ -e "$ledger" ] || : >"$ledger"
  json="$(mktemp "${TMPDIR:-/tmp}/stackstead-demo-create.XXXXXX")"
  if ! (cd "$example_root" && "$stackstead_bin" --json create "$name") >"$json"; then
    rm -f "$json"
    die "failed to create $name; retained ledger lists earlier stacksteads for explicit cleanup"
  fi
  jq -e '.kind == "StacksteadChange" and .version == "1" and .action == "created"' \
    "$json" >/dev/null || die "unsupported Stackstead create response for $name"
  CREATED_ID="$(jq -er '.stackstead.stackstead_id' "$json")"
  CREATED_WORKTREE="$(jq -er '.stackstead.worktree' "$json")"
  CREATED_MANIFEST="$(jq -er '.stackstead.files.manifest' "$json")"
  CREATED_PROJECT="$(jq -er '.stackstead.compose_project' "$json")"
  printf '%s\t%s\t%s\t%s\t%s\n' \
    "$name" "$CREATED_ID" "$CREATED_WORKTREE" "$CREATED_MANIFEST" "$CREATED_PROJECT" >>"$ledger"
  rm -f "$json"
}

create_three() {
  require_runtime
  [ ! -e "$ledger" ] || die "demo ledger already exists; run '$0 cleanup' first"
  : >"$ledger"
  for agent in alpha beta gamma; do
    payload="$(payload_for_agent "$agent")"
    register_cell "$agent"
    write_branch_contract "$CREATED_WORKTREE" "$agent" "$payload"
    (cd "$example_root" && "$stackstead_bin" up "$CREATED_ID")
  done
  printf 'Created and started three stacksteads. Ledger: %s\n' "$ledger"
}

compose_for() {
  manifest="$1"
  shift
  project="$(jq -er '.compose_project' "$manifest")"
  env_file="$(jq -er '.env_file' "$manifest")"
  compose_file="$(jq -er '.compose_files[0]' "$manifest")"
  worktree="$(jq -er '.worktree' "$manifest")"
  ownership_override="$worktree/.stackstead/compose-ownership.yaml"
  docker compose --project-name "$project" --env-file "$env_file" \
    -f "$compose_file" -f "$ownership_override" "$@"
}

assert_project_ports_loopback() {
  project="$1"
  containers="$(docker ps -q --filter "label=com.docker.compose.project=$project")"
  [ -n "$containers" ] || die "expected containers for $project"
  docker inspect $containers | jq -e '
    [.[].NetworkSettings.Ports[]?[]?.HostIp]
    | length > 0 and all(. == "127.0.0.1" or . == "::1")
  ' >/dev/null || die "published port escaped loopback for $project"
}

query_contract() {
  manifest="$1"
  id="$(jq -er '.stackstead_id' "$manifest")"
  (cd "$example_root" && "$stackstead_bin" exec "$id" postgres -- \
    psql -XAt -U app -d app \
    -c "SELECT migration_id || '|' || agent || '|' || payload FROM schema_migrations;")
}

ledger_field() {
  agent="$1"
  field="$2"
  awk -F '\t' -v agent="$agent" -v field="$field" '$1 == agent { print $field }' "$ledger"
}

verify_three() {
  require_runtime
  require curl
  [ -s "$ledger" ] || die "demo ledger not found; run '$0 create' first"
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/stackstead-demo-verify.XXXXXX")"
  while IFS=$'\t' read -r agent id worktree manifest project <&3; do
    [ -f "$manifest" ] || die "missing manifest for $id: $manifest"
    [ "$(jq -er '.stackstead_id' "$manifest")" = "$id" ] || die "manifest ID mismatch for $id"
    [ "$(jq -er '.worktree' "$manifest")" = "$worktree" ] || die "worktree mismatch for $id"
    [ "$(jq -er '.slug' "$manifest")" = "$agent" ] || die "slug mismatch for $id"
    jq -r '.compose_project' "$manifest" >>"$tmp/projects"
    jq -r '.ports[]' "$manifest" >>"$tmp/ports"
    [ "$(jq -er '.compose_project' "$manifest")" = "$project" ] ||
      die "Compose project mismatch for $id"
    inspection="$(cd "$example_root" && "$stackstead_bin" --json inspect "$id")"
    jq -e '
      .kind == "StacksteadInspection" and .version == "3" and
      any(.live.services[]; .service == "setup" and .status == "completed (0)")
    ' <<<"$inspection" >/dev/null || die "inspect did not report the successful setup job for $id"
    container="$(compose_for "$manifest" ps -q postgres)"
    [ -n "$container" ] || die "Postgres is not running for $id"
    label="$(docker inspect -f '{{ index .Config.Labels "com.docker.compose.project" }}' "$container")"
    [ "$label" = "$project" ] || die "container project label mismatch for $id"
    assert_project_ports_loopback "$project"
    result="$(query_contract "$manifest")"
    expected="202607090001|$agent|$(payload_for_agent "$agent")"
    [ "$result" = "$expected" ] || die "database contract mismatch for $id: $result"
    owner="$(cd "$example_root" && "$stackstead_bin" exec "$id" postgres -- psql -XAt -U app -d app -c 'SELECT owner FROM seeded_accounts WHERE account_id = 1;')"
    [ "$owner" = "$agent" ] || die "seed leaked or is missing for $id: $owner"
    url="$(jq -er '.urls.web' "$manifest")"
    page="$(curl --fail --silent --show-error --retry 10 --retry-connrefused --retry-delay 1 "$url")"
    case "$page" in
      *'Stackstead three-agent isolation proof'*) ;;
      *) die "web assertion failed for $id at $url" ;;
    esac
    printf 'PASS identity=%s project=%s database_agent=%s url=%s\n' "$id" "$project" "$agent" "$url"
  done 3<"$ledger"
  [ "$(sort -u "$tmp/projects" | wc -l)" -eq 3 ] || die "Compose projects are not unique"
  [ "$(sort -u "$tmp/ports" | wc -l)" -eq 6 ] || die "host ports are not unique"
  rm -rf "$tmp"
  printf 'PASS all three manifests, Compose projects, six ports, databases, seeds, and URLs are isolated.\n'
}

crash_beta() {
  require_runtime
  [ -s "$ledger" ] || die "demo ledger not found; run '$0 create' first"
  beta_manifest="$(ledger_field beta 4)"
  [ -f "$beta_manifest" ] || die "beta manifest is missing"
  compose_for "$beta_manifest" kill postgres >/dev/null
  if query_contract "$beta_manifest" >/dev/null 2>&1; then
    die "beta database still accepts queries after kill"
  fi
  for agent in alpha gamma; do
    manifest="$(ledger_field "$agent" 4)"
    result="$(query_contract "$manifest")"
    case "$result" in
      "202607090001|$agent|"*) ;;
      *) die "$agent changed when beta crashed: $result" ;;
    esac
  done
  printf 'PASS beta Postgres was killed by exact project identity; alpha and gamma remained queryable.\n'
}

recover_beta() {
  require_runtime
  [ -s "$ledger" ] || die "demo ledger not found; run '$0 create' first"
  beta_id="$(ledger_field beta 2)"
  beta_manifest="$(ledger_field beta 4)"
  [ -n "$beta_id" ] && [ -f "$beta_manifest" ] || die "beta ledger entry is incomplete"
  (cd "$example_root" && "$stackstead_bin" up "$beta_id")
  result="$(query_contract "$beta_manifest")"
  [ "$result" = "202607090001|beta|$(payload_for_agent beta)" ] ||
    die "beta data did not survive recovery: $result"
  printf 'PASS beta recovered under the same manifest and retained its branch-local data.\n'
}

create_retired_alpha_service() {
  require_runtime
  [ -s "$ledger" ] || die "demo ledger not found; run '$0 create' first"
  manifest="$(ledger_field alpha 4)"
  worktree="$(ledger_field alpha 3)"
  compose_file="$(jq -er '.compose_files[0]' "$manifest")"
  project="$(jq -er '.compose_project' "$manifest")"
  runtime_token="$(jq -er '.runtime_token' "$manifest")"
  retired_ownership="$worktree/.stackstead/retired-ownership.yaml"
  awk '
    !inserted && /^volumes:/ {
      print "  retired-worker:"
      print "    image: nginx:1.27-alpine"
      print "    command: [\"sh\", \"-c\", \"sleep 3600\"]"
      print ""
      inserted = 1
    }
    { print }
    END { if (!inserted) exit 2 }
  ' "$compose_file" >"$compose_file.next" || die "could not create orphan-service fixture"
  mv "$compose_file.next" "$compose_file"
  printf 'services:\n  retired-worker:\n    labels:\n      io.stackstead.runtime-token: "%s"\n' \
    "$runtime_token" >"$retired_ownership"
  compose_for "$manifest" -f "$retired_ownership" up -d retired-worker >/dev/null
  container="$(compose_for "$manifest" -f "$retired_ownership" ps -q retired-worker)"
  [ -n "$container" ] || die "retired alpha service did not start"
  label="$(docker inspect -f '{{ index .Config.Labels "com.docker.compose.project" }}' "$container")"
  [ "$label" = "$project" ] || die "retired service project label mismatch"
  git -C "$worktree" checkout -- docker-compose.yml
  rm -f "$retired_ownership"
  docker run --rm --user 0:0 --mount "type=bind,src=$worktree,dst=/source" \
    alpine@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc \
    sh -ceu 'mkdir -p /source/.stackstead/container-owned; printf generated > /source/.stackstead/container-owned/artifact; chmod 400 /source/.stackstead/container-owned/artifact; chmod 500 /source/.stackstead/container-owned'
  [ -z "$(git -C "$worktree" status --porcelain=v1 --untracked-files=all)" ] ||
    die "alpha worktree is dirty after restoring the current Compose contract"
  printf 'PASS alpha retains one retired service and one non-writable container-created source artifact for exact cleanup.\n'
}

docker_inventory() {
  kind="$1"
  shift
  case "$kind" in
    containers) docker ps -aq "$@" | sort ;;
    volumes) docker volume ls -q "$@" | sort ;;
    networks) docker network ls -q "$@" | sort ;;
    *) die "unknown Docker inventory kind: $kind" ;;
  esac
}

cleanup() {
  require_runtime
  if [ ! -s "$ledger" ]; then
    printf 'No demo ledger; nothing to destroy.\n'
    rm -f "$ledger"
    return
  fi
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/stackstead-demo-cleanup.XXXXXX")"
  for kind in containers volumes networks; do
    docker_inventory "$kind" >"$tmp/$kind.before"
    : >"$tmp/$kind.target"
  done
  while IFS=$'\t' read -r _ _ _ manifest project <&3; do
    [ -n "$project" ] || die "refusing cleanup: ledger entry has no Compose project"
    if [ -f "$manifest" ]; then
      [ "$(jq -er '.compose_project' "$manifest")" = "$project" ] ||
        die "refusing cleanup: manifest project mismatch for $manifest"
    fi
    for kind in containers volumes networks; do
      docker_inventory "$kind" --filter "label=com.docker.compose.project=$project" >>"$tmp/$kind.target"
    done
  done 3<"$ledger"
  for kind in containers volumes networks; do
    sort -u "$tmp/$kind.target" -o "$tmp/$kind.target"
    comm -23 "$tmp/$kind.before" "$tmp/$kind.target" >"$tmp/$kind.nontarget.before"
  done
  while IFS=$'\t' read -r agent id _ manifest project <&3; do
    if [ -f "$manifest" ]; then
      [ "$(jq -er '.stackstead_id' "$manifest")" = "$id" ] || die "refusing cleanup: manifest ID mismatch for $id"
      [ "$(jq -er '.compose_project' "$manifest")" = "$project" ] || die "refusing cleanup: manifest project mismatch for $id"
      (cd "$example_root" && "$stackstead_bin" destroy "$id" --yes)
    elif [ -n "$(docker ps -aq --filter "label=com.docker.compose.project=$project")$(docker volume ls -q --filter "label=com.docker.compose.project=$project")$(docker network ls -q --filter "label=com.docker.compose.project=$project")" ]; then
      die "manifest missing for $id while exact project resources still exist; retained ledger for recovery"
    fi
    [ -z "$(docker ps -aq --filter "label=com.docker.compose.project=$project")" ] ||
      die "target containers remain for $project"
    [ -z "$(docker volume ls -q --filter "label=com.docker.compose.project=$project")" ] ||
      die "target volumes remain for $project"
    [ -z "$(docker network ls -q --filter "label=com.docker.compose.project=$project")" ] ||
      die "target networks remain for $project"
    awk -F '\t' -v id="$id" '$2 != id' "$ledger" >"$ledger.next"
    mv "$ledger.next" "$ledger"
    printf 'PASS destroyed ledger identity=%s agent=%s project=%s\n' "$id" "$agent" "$project"
  done 3<"$ledger"
  for kind in containers volumes networks; do
    docker_inventory "$kind" >"$tmp/$kind.after"
    comm -23 "$tmp/$kind.nontarget.before" "$tmp/$kind.after" >"$tmp/$kind.nontarget.removed"
    [ ! -s "$tmp/$kind.nontarget.removed" ] ||
      die "non-demo Docker $kind disappeared during cleanup; rerun on a quiescent Docker daemon"
  done
  rm -f "$ledger"
  rm -rf "$tmp"
  printf 'PASS cleanup issued only manifest-led Stackstead destroy operations; no global prune was used.\n'
  printf 'Note: removed inventory includes the recorded demo projects by design; run against a quiescent Docker daemon for strict inventory attribution.\n'
}

assert_project_runtime_exists() {
  project="$1"
  [ -n "$(docker ps -q --filter "label=com.docker.compose.project=$project")" ] ||
    die "expected running containers for $project"
  [ -n "$(docker volume ls -q --filter "label=com.docker.compose.project=$project")" ] ||
    die "expected a managed volume for $project"
}

fixed_port_negative() {
  require_runtime
  require curl
  [ ! -e "$ledger" ] || die "fixed-port negative requires an empty demo ledger"
  compose_file="$example_root/docker-compose.yml"
  sed 's/"127.0.0.1:${WEB_PORT}:80"/"8080:80"/' "$compose_file" >"$compose_file.next"
  mv "$compose_file.next" "$compose_file"
  grep -q '"8080:80"' "$compose_file" || die "could not create fixed-port fixture"
  git -C "$example_root" add docker-compose.yml
  git -C "$example_root" \
    -c user.name='Stackstead Demo' \
    -c user.email='stackstead-demo.invalid' \
    commit -m 'Create fixed-port negative fixture' >/dev/null

  fixed_tmp="$(mktemp -d "${TMPDIR:-/tmp}/stackstead-fixed-negative.XXXXXX")"
  for kind in containers volumes networks; do
    docker_inventory "$kind" >"$fixed_tmp/$kind.before"
  done
  if (cd "$example_root" && "$stackstead_bin" --json create fixed-reject) \
    >"$fixed_tmp/create.out" 2>"$fixed_tmp/create.err"; then
    create_status=0
  else
    create_status=$?
  fi
  cat "$fixed_tmp/create.out" "$fixed_tmp/create.err"
  [ "$create_status" -ne 0 ] || die "fixed host port unexpectedly passed structural validation"
  for kind in containers volumes networks; do
    docker_inventory "$kind" >"$fixed_tmp/$kind.after"
    cmp "$fixed_tmp/$kind.before" "$fixed_tmp/$kind.after" || die "fixed-port rejection changed $kind"
  done
  [ -z "$(git -C "$example_root" branch --list fixed-reject)" ] ||
    die "fixed-port rejection left a branch"
  [ -z "$(find "$state_root/stackstead-three-agent-proof" -mindepth 1 -maxdepth 1 -name 'fixed-reject-*' -print 2>/dev/null)" ] ||
    die "fixed-port rejection left Stackstead state"
  (cd "$example_root" && "$stackstead_bin" ps --json) |
    jq -e 'all(.stacksteads[]; .branch != "fixed-reject")' >/dev/null ||
    die "fixed-port rejection registered a Stackstead"
  rm -f "$ledger"

  (cd "$example_root" && "$stackstead_bin" compose apply --yes)
  git -C "$example_root" add docker-compose.yml
  git -C "$example_root" \
    -c user.name='Stackstead Demo' \
    -c user.email='stackstead-demo.invalid' \
    commit -m 'Apply generated Stackstead port mapping' >/dev/null

  register_cell fixed-a
  (cd "$example_root" && "$stackstead_bin" up "$CREATED_ID")
  first_url="$(jq -er '.urls.web' "$CREATED_MANIFEST")"
  register_cell fixed-b
  (cd "$example_root" && "$stackstead_bin" up "$CREATED_ID")
  second_url="$(jq -er '.urls.web' "$CREATED_MANIFEST")"
  assert_project_ports_loopback "$(ledger_field fixed-a 5)"
  assert_project_ports_loopback "$(ledger_field fixed-b 5)"
  [ "$first_url" != "$second_url" ] || die "applied fixed-port stacksteads reused a URL"
  curl --fail --silent --show-error --retry 10 --retry-connrefused "$first_url" >/dev/null
  curl --fail --silent --show-error --retry 10 --retry-connrefused "$second_url" >/dev/null
  cleanup
  rm -rf "$fixed_tmp"
  printf 'PASS fixed host port failed before Docker mutation, then explicit apply produced two isolated live URLs.\n'
}

dirty_destruction_negative() {
  require_runtime
  [ ! -e "$ledger" ] || die "dirty-source negative requires an empty demo ledger"
  register_cell dirty-source
  (cd "$example_root" && "$stackstead_bin" up "$CREATED_ID")
  assert_project_runtime_exists "$CREATED_PROJECT"

  printf 'staged change\n' >>"$CREATED_WORKTREE/README.md"
  git -C "$CREATED_WORKTREE" add README.md
  printf 'unstaged change\n' >>"$CREATED_WORKTREE/README.md"
  printf 'untracked change\n' >"$CREATED_WORKTREE/dirty-untracked.txt"
  if (cd "$example_root" && "$stackstead_bin" destroy "$CREATED_ID" --yes); then
    destroy_status=0
  else
    destroy_status=$?
  fi
  [ "$destroy_status" -ne 0 ] || die "dirty destroy unexpectedly succeeded"
  [ -f "$CREATED_MANIFEST" ] || die "dirty destroy removed the manifest"
  [ -d "$CREATED_WORKTREE" ] || die "dirty destroy removed the source checkout"
  assert_project_runtime_exists "$CREATED_PROJECT"
  git -C "$CREATED_WORKTREE" diff --cached --quiet && die "staged change disappeared"
  git -C "$CREATED_WORKTREE" diff --quiet && die "unstaged change disappeared"
  grep -q '^staged change$' "$CREATED_WORKTREE/README.md" || die "staged content disappeared"
  grep -q '^unstaged change$' "$CREATED_WORKTREE/README.md" || die "unstaged content disappeared"
  [ "$(cat "$CREATED_WORKTREE/dirty-untracked.txt")" = "untracked change" ] ||
    die "untracked content disappeared"

  git -C "$CREATED_WORKTREE" reset --hard HEAD >/dev/null
  rm -f "$CREATED_WORKTREE/dirty-untracked.txt"
  cleanup
  printf 'PASS tracked, staged, and untracked dirtiness failed closed with runtime and volume intact.\n'
}

corrupt_state_negative() (
  require_runtime
  [ ! -e "$ledger" ] || die "corrupt-state negative requires an empty demo ledger"
  victim_container=
  corrupt_tmp=
  manifest_backup=
  pointer_backup=
  restore_corrupt_fixture() {
    [ -z "$manifest_backup" ] || [ ! -f "$manifest_backup" ] ||
      cp "$manifest_backup" "$CREATED_MANIFEST"
    [ -z "$pointer_backup" ] || [ ! -f "$pointer_backup" ] ||
      cp "$pointer_backup" "$CREATED_WORKTREE/.stackstead/stackstead.json"
    [ -z "$victim_container" ] || docker rm -f "$victim_container" >/dev/null 2>&1 || true
    [ -z "$corrupt_tmp" ] || rm -rf "$corrupt_tmp"
  }
  trap restore_corrupt_fixture EXIT
  register_cell corrupt-state
  (cd "$example_root" && "$stackstead_bin" up "$CREATED_ID")
  assert_project_runtime_exists "$CREATED_PROJECT"

  victim_project="stackstead-negative-victim-$$"
  victim_container="$victim_project-container"
  docker run -d --name "$victim_container" \
    --label "com.docker.compose.project=$victim_project" \
    nginx:1.27-alpine >/dev/null
  corrupt_tmp="$(mktemp -d "${TMPDIR:-/tmp}/stackstead-corrupt-negative.XXXXXX")"
  manifest_backup="$corrupt_tmp/manifest.json"
  pointer_backup="$corrupt_tmp/pointer.json"
  cp "$CREATED_MANIFEST" "$manifest_backup"
  cp "$CREATED_WORKTREE/.stackstead/stackstead.json" "$pointer_backup"

  jq --arg project "$victim_project" '.compose_project = $project' \
    "$manifest_backup" >"$CREATED_MANIFEST"
  if (cd "$example_root" && "$stackstead_bin" destroy "$CREATED_ID" --yes); then
    project_status=0
  else
    project_status=$?
  fi
  cp "$manifest_backup" "$CREATED_MANIFEST"
  [ "$project_status" -ne 0 ] || die "tampered Compose project unexpectedly destroyed"
  assert_project_runtime_exists "$CREATED_PROJECT"
  docker inspect "$victim_container" >/dev/null

  jq --arg path "$example_root" '.worktree = $path' \
    "$manifest_backup" >"$CREATED_MANIFEST"
  if (cd "$example_root" && "$stackstead_bin" destroy "$CREATED_ID" --yes); then
    path_status=0
  else
    path_status=$?
  fi
  cp "$manifest_backup" "$CREATED_MANIFEST"
  [ "$path_status" -ne 0 ] || die "tampered worktree path unexpectedly destroyed"
  assert_project_runtime_exists "$CREATED_PROJECT"
  docker inspect "$victim_container" >/dev/null

  jq '.stackstead_id = "forged-pointer-a123"' \
    "$pointer_backup" >"$CREATED_WORKTREE/.stackstead/stackstead.json"
  if (cd "$CREATED_WORKTREE" && "$stackstead_bin" destroy "$CREATED_ID" --yes); then
    pointer_status=0
  else
    pointer_status=$?
  fi
  cp "$pointer_backup" "$CREATED_WORKTREE/.stackstead/stackstead.json"
  [ "$pointer_status" -ne 0 ] || die "tampered pointer unexpectedly destroyed"
  assert_project_runtime_exists "$CREATED_PROJECT"
  docker inspect "$victim_container" >/dev/null

  cleanup
  docker inspect "$victim_container" >/dev/null || die "exact cleanup removed the unrelated victim"
  docker rm -f "$victim_container" >/dev/null
  victim_container=
  rm -rf "$corrupt_tmp"
  corrupt_tmp=
  manifest_backup=
  pointer_backup=
  printf 'PASS project, worktree-path, and pointer corruption failed closed; intended and victim resources survived.\n'
)

run_negatives() {
  fixed_port_negative
  dirty_destruction_negative
  corrupt_state_negative
  printf 'PASS all live destructive negative cases completed.\n'
}

run_all() {
  [ "$#" -eq 1 ] || die "usage: $0 all <empty-destination>"
  destination="$1"
  prepare "$destination"
  copied="$destination/demo.sh"
  for phase in create verify crash recover verify orphan cleanup; do
    STACKSTEAD_BIN="$stackstead_bin" "$copied" "$phase"
  done
}

usage() {
  cat <<EOF
Usage:
  $0 prepare <empty-destination>
  $0 create | verify | crash | recover | orphan | cleanup | negatives
  $0 all <empty-destination>

The phase commands run from a prepared copy. 'all' keeps the copied Git repository
but destroys only the three manifest-recorded Stackstead runtimes, networks, volumes,
and project-labeled orphan services.
EOF
}

command_name="${1:-}"
if [ "$#" -gt 0 ]; then
  shift
fi
case "$command_name" in
  prepare) prepare "$@" ;;
  create) [ "$#" -eq 0 ] || die "create takes no arguments"; create_three ;;
  verify) [ "$#" -eq 0 ] || die "verify takes no arguments"; verify_three ;;
  crash) [ "$#" -eq 0 ] || die "crash takes no arguments"; crash_beta ;;
  recover) [ "$#" -eq 0 ] || die "recover takes no arguments"; recover_beta ;;
  orphan) [ "$#" -eq 0 ] || die "orphan takes no arguments"; create_retired_alpha_service ;;
  cleanup) [ "$#" -eq 0 ] || die "cleanup takes no arguments"; cleanup ;;
  negatives) [ "$#" -eq 0 ] || die "negatives takes no arguments"; run_negatives ;;
  all) run_all "$@" ;;
  -h | --help | help | '') usage ;;
  *) usage >&2; die "unknown command: $command_name" ;;
esac
