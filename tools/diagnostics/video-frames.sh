#!/bin/bash
# Collect the frame-presentation counters from a completed Quickie playback run.
set -euo pipefail

if [[ $# != 3 || ! $1 =~ ^[0-9]+$ || ! $2 =~ ^[0-9]+$ ]]; then
    printf 'Usage: %s QUICKIE_PID GPU_PID "YYYY-MM-DD HH:MM:SS"\n' "$0" >&2
    exit 2
fi

if [[ $(uname -s) != Darwin ]]; then
    printf 'Video presentation counters require the macOS unified log.\n' >&2
    exit 1
fi

predicate="processIdentifier IN {$1,$2} AND (eventMessage CONTAINS 'recent frames' OR eventMessage CONTAINS 'Enqueued Pixel Buffer' OR eventMessage CONTAINS 'view visibility state changed')"
exec /usr/bin/log show --start "$3" --style compact --predicate "$predicate"
