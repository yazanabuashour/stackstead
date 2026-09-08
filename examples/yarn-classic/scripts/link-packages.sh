#!/bin/sh
set -eu

: "${STACKSTEAD_WORKTREE:?Run this script through Stackstead}"
if [ "$(pwd -P)" != "$STACKSTEAD_WORKTREE" ]; then
  printf '%s\n' 'Refusing to install outside STACKSTEAD_WORKTREE.' >&2
  exit 1
fi

# Never follow a link into another environment or the global Yarn registry.
for directory in "$STACKSTEAD_WORKTREE/.stackstead" "$STACKSTEAD_WORKTREE/.stackstead/yarn-links"; do
  if [ -L "$directory" ] || { [ -e "$directory" ] && [ ! -d "$directory" ]; }; then
    printf 'Refusing unsafe Yarn link directory: %s\n' "$directory" >&2
    exit 1
  fi
  mkdir -p "$directory"
done
export YARN_LINK_FOLDER="$STACKSTEAD_WORKTREE/.stackstead/yarn-links"

yarn install --frozen-lockfile --link-folder "$YARN_LINK_FOLDER"

# This example has no packages to link. Add repository-specific registration and
# consumer commands here, always using yarn link --link-folder "$YARN_LINK_FOLDER".
printf 'Repository-local Yarn link folder is ready: %s\n' "$YARN_LINK_FOLDER"
