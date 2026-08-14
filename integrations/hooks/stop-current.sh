#!/usr/bin/env bash
set -Eeuo pipefail

stackstead_bin="${STACKSTEAD_BIN:-stackstead}"
id="$("$stackstead_bin" current)"
"$stackstead_bin" stop "$id"
