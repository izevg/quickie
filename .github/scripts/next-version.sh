#!/usr/bin/env bash
# Prints the next release version (no leading "v") for the repository in the current directory,
# or nothing when the commits since the last `v*` tag hold nothing to release.
#   - rules and the first version (0.1.0) live in .github/cliff.toml;
#   - git-cliff must be on PATH (mise provides it), or set CLIFF to the command to run.
# Usage: .github/scripts/next-version.sh
set -eu

config="$(dirname "$0")/../cliff.toml"
cliff() { ${CLIFF:-git-cliff} --config "$config" "$@"; }

# `--bumped-version` alone cannot say "no release": with no tag it answers the initial version,
# and with only docs commits it repeats the last tag. The notes are empty exactly when every
# commit since the last tag is a type that never releases (docs, chore, ci, ...).
notes="$(cliff --unreleased --strip all 2>/dev/null)"
if [ -z "$(printf '%s' "$notes" | tr -d '[:space:]')" ]; then
  exit 0
fi

version="$(cliff --bumped-version 2>/dev/null)"
printf '%s\n' "${version#v}"
