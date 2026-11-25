#!/bin/bash
# Compare the reference and current link collectors on one pinned macOS WebKit build.
set -euo pipefail

mode=${1:-}
case "$mode" in
    --gate|--release) shift ;;
    *) mode= ;;
esac
if [[ $# != 1 && $# != 2 ]]; then
    printf 'Usage: %s [--gate|--release] WEBKIT_RELEASE_DIRECTORY [SAMPLES_PER_VARIANT]\n' "$0" >&2
    exit 2
fi
if [[ $(uname -s) != Darwin ]]; then
    printf 'The native link benchmark needs macOS.\n' >&2
    exit 1
fi

samples=${2:-24}
if [[ ! $samples =~ ^[1-9][0-9]?$ ]] || ((samples < 5 || samples > 80)); then
    printf 'Use 5 to 80 samples.\n' >&2
    exit 2
fi
if [[ -n $mode ]] && ((samples < 20 || samples % 2 != 0)); then
    printf 'Use an even number of samples from 20 to 80 for the performance gate.\n' >&2
    exit 2
fi
frameworks=$(cd "$1" && pwd -P)
framework="$frameworks/WebKit.framework/WebKit"
if [[ ! -f "$framework" ]]; then
    printf 'No WebKit.framework in %s.\n' "$frameworks" >&2
    exit 2
fi

policy=$(cd "$(dirname "$0")/agent-links" && pwd -P)/gate.jq
cd "$(git rev-parse --show-toplevel)"
cargo_command=(cargo)
if [[ -x "$HOME/orca/bin/heavy" ]]; then
    cargo_command=("$HOME/orca/bin/heavy" "${cargo_command[@]}")
fi
binary=$(
    CARGO_PROFILE_RELEASE_STRIP=none "${cargo_command[@]}" test --locked --release \
        -p quickie-engine-webkit --test conformance --no-run --message-format=json-render-diagnostics |
    jq -esr '[.[] | select(.reason == "compiler-artifact" and .target.name == "conformance"
        and .executable != null).executable]
        | if length == 1 then .[0] else error("expected one conformance executable from Cargo") end'
)

os=$(sw_vers -productVersion)
cpu=$(sysctl -n machdep.cpu.brand_string)
uuid=$(xcrun dwarfdump --uuid "$framework" | awk '$3 == "(arm64)" {print $2, $3}')
framework=$(realpath "$framework")

# Launch separate processes from the same optimized binary. Cargo and the normal
# conformance tests have finished before the timing process starts.
export DYLD_FRAMEWORK_PATH="$frameworks" DYLD_LIBRARY_PATH="$frameworks"
export __XPC_DYLD_FRAMEWORK_PATH="$frameworks" __XPC_DYLD_LIBRARY_PATH="$frameworks"
if [[ "$mode" == --release ]]; then
    "$binary"
fi
report=$(QUICKIE_PERF_OS="$os" QUICKIE_PERF_CPU="$cpu" QUICKIE_PERF_FRAMEWORK_UUID="$uuid" \
    "$binary" "--agent-links-perf=$samples") || {
    status=$?
    printf '%s\n' "$report" >&2
    exit "$status"
}
printf '%s\n' "$report"
if [[ -z "$mode" ]]; then
    exit 0
fi

result=$(jq -esc --arg framework "$framework" --arg uuid "$uuid" --argjson samples "$samples" \
    -f "$policy" <<< "$report")
summary=$(jq -r '
    def display: . * 100 | round / 100;
    [
        "### Agent link performance",
        "",
        "macOS \(.macos) · \(.cpu) · WebKit \(.framework_uuid) · \(.samples_per_variant) sample pairs",
        "",
        "| Workload | Stage | Median before → after (µs) | p95 before → after (µs) | Median ratio | Limit | Result |",
        "|---|---|---:|---:|---:|---:|---|",
        (.measurements[] |
            "| \(.workload) | \(.stage) | \(.before.p50 | display) → \(.after.p50 | display) | \(.before.p95 | display) → \(.after.p95 | display) | \(.ratio)× | \(.limit)× | \(if .passed then "Pass" else "Fail" end) |")
    ][]' <<< "$result")
printf '%s\n' "$summary" >&2
if [[ -n ${GITHUB_STEP_SUMMARY:-} ]]; then
    printf '%s\n' "$summary" >> "$GITHUB_STEP_SUMMARY"
fi
if ! jq -e '.passed' <<< "$result" >/dev/null; then
    printf 'Agent link performance exceeded its release budget.\n' >&2
    exit 1
fi
