#!/bin/sh
set -eu
state='@STATE@'
project='@PROJECT@'
token='@TOKEN@'
base='@COMPOSE_BASE@'
metadata='@METADATA_TEMPLATE@'
claim="$project-stackstead-claim"
printf '%s\n' "$*" >> "$state/commands"
reject() {
  printf '%s\n' "$*" >> "$state/unexpected"
  exit 97
}
test "$COMPOSE_PROJECT_NAME" = "$project" || reject 'wrong project environment'
case "$*" in
  "container ls --all --format {{.Names}}")
    while read -r id name project_label; do printf '%s\n' "$name"; done < "$state/containers"
    exit 0 ;;
  "container ls --all --filter label=com.docker.compose.project=$project --format {{.ID}}"|\
  "container ls --all --filter label=com.docker.compose.project=$project --format {{.ID}} --no-trunc")
    while read -r id name project_label; do
      if test "$project_label" = "$project"; then printf '%s\n' "$id"; fi
    done < "$state/containers"
    exit 0 ;;
  "network ls --format {{.Name}}"|\
  "network ls --filter label=com.docker.compose.project=$project --format {{.ID}}") exit 0 ;;
  "volume ls --format {{.Name}}"|\
  "volume ls --filter label=com.docker.compose.project=$project --format {{.Name}}")
    if test -f "$state/claim"; then printf '%s\n' "$claim"; fi
    exit 0 ;;
  "volume create --label com.docker.compose.project=$project --label io.stackstead.runtime-token=$token $claim")
    printf '%s' "$token" > "$state/claim"
    printf '%s\n' "$claim"
    exit 0 ;;
  "volume inspect --format {{json .Labels}} $claim")
    test -f "$state/claim" || exit 41
    printf '{"io.stackstead.runtime-token":"%s"}\n' "$(cat "$state/claim")"
    exit 0 ;;
  "$base config --format json"|"$base config --hash *")
    test ! -f "$state/fail-config" || exit 42
    test "${COMPOSE_PROFILES-<absent>}" = "$(cat "$state/expected-profiles")" || reject 'profile selection was not pinned'
    if test "$*" = "$base config --format json"; then cat "$state/model.json"; else cat "$state/hashes"; fi
    exit 0 ;;
  "$base up -d")
    test ! -f "$state/fail-up" || exit 43
    touch "$state/up-ran"
    exit 0 ;;
  "$base ps --status running --quiet") cat "$state/running"; exit 0 ;;
  "$base port postgres 5432")
    host=${PUBLICATION_HOST-127.0.0.1}
    if test -n "${PUBLICATION_PROBE_RECEIPT-}"; then
      if test -e "$PUBLICATION_PROBE_RECEIPT"; then host=127.0.0.2; else touch "$PUBLICATION_PROBE_RECEIPT"; fi
    fi
    printf '%s:@DATABASE_PORT@\n' "$host"
    exit 0 ;;
esac
if test "$#" = 5 && test "$1 $2 $3" = 'container inspect --format'; then
  while read -r id name project_label; do
    if test "$5" = "$id" || test "$5" = "$name"; then
      case "$4" in
        '{{json .Config.Labels}}')
          cat "$state/$id.labels"
          exit 0 ;;
        "$metadata")
          test "$5" = "$id" || reject 'metadata inspection did not use immutable ID'
          test ! -f "$state/disappear" || exit 44
          cat "$state/$id"
          exit 0 ;;
      esac
    fi
  done < "$state/containers"
fi
reject "$*"
