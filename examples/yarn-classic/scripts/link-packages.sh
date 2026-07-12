#!/bin/sh
set -eu

: "${YARN_LINK_FOLDER:?Stackstead must set YARN_LINK_FOLDER}"
mkdir -p "$YARN_LINK_FOLDER"
printf 'Stackstead-local Yarn link folder is ready.\n' >"$YARN_LINK_FOLDER/example-link.txt"
