#!/bin/sh
set -eu
directory=$1
exec 3<&0

# Explicit stdin defeats the shell's /dev/null default for background commands.
# EOF is the failure-cleanup barrier; no helper needs a saved PID or external sleep.
sh -c '
    trap '\'' : > "$1/member-term"; exit 0 '\'' TERM
    : > "$1/member-ready"
    IFS= read -r release || :
' original-group-member "$directory" <&3 &
member=$!

# Reap the member before publishing the leader receipt. Only the owning shell waits.
trap 'wait "$member" || :; : > "$directory/leader-term"; exit 0' TERM
: > "$directory/leader-ready"
wait "$member" || :
