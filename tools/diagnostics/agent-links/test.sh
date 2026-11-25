#!/bin/bash
set -euo pipefail

gate=$(cd "$(dirname "$0")" && pwd -P)/gate.jq
framework=/fixture/WebKit.framework/WebKit
uuid='00000000-0000-0000-0000-000000000001 (arm64)'
report=$(jq -n --arg framework "$framework" --arg uuid "$uuid" '
    def timing($before; $after):
        {before: {p50: $before, p95: ($before * 1.5)}, after: {p50: $after, p95: ($after * 1.5)}};
    {
      benchmark: "agent-links",
      baseline: "48501264252595d7d98a0aec62da60c1f030bbf9",
      framework: $framework,
      framework_uuid: $uuid,
      architecture: "aarch64",
      macos: "26.5.2",
      cpu: "Apple M1",
      samples_per_variant: 24,
      warmup_pairs: 4,
      webkit_iterations_per_sample: 32,
      rust_iterations_per_sample: 128,
      microseconds_per_call: {
        typical: {webkit: timing(100; 98), rust: timing(100; 77)},
        deep: {webkit: timing(600; 510), rust: timing(100; 77)}
      }
    }')

evaluate() {
    jq -esc --arg framework "$framework" --arg uuid "$uuid" --argjson samples 24 -f "$gate"
}

# A passing report includes all four comparisons and retains both percentiles.
accepted=$(evaluate <<< "$report")
jq -e '.passed and (.measurements | length == 4)
    and all(.measurements[]; (.before.p95 > .before.p50 and .after.p95 > .after.p50))' \
    <<< "$accepted" >/dev/null

# A deep collector regression fails the aggregate verdict, while retaining diagnostics.
regression=$(jq '.microseconds_per_call.deep.webkit.after.p50 = 600' <<< "$report")
evaluated=$(evaluate <<< "$regression")
jq -e '(.passed | not) and ([.measurements[] | select(.passed | not) | [.workload, .stage]]
    == [["deep", "webkit"]])' <<< "$evaluated" >/dev/null

# The limit is inclusive, and a regression in the Rust parser also blocks release.
boundary=$(jq '.microseconds_per_call.typical.rust.after.p50 = 96' <<< "$report")
evaluate <<< "$boundary" | jq -e '.passed' >/dev/null
rust_regression=$(jq '.microseconds_per_call.typical.rust.after.p50 = 97' <<< "$report")
evaluate <<< "$rust_regression" | jq -e '.passed | not' >/dev/null

reject() {
    if evaluate <<< "$1" >/dev/null 2>&1; then
        printf 'Expected an invalid performance report to be rejected.\n' >&2
        exit 1
    fi
}

reject ""
reject '{'
reject "$report"$'\n'"$report"
reject "$(jq 'del(.microseconds_per_call.deep)' <<< "$report")"
reject "$(jq '.microseconds_per_call.typical.rust.before.p50 = 0' <<< "$report")"
reject "$(jq '.microseconds_per_call.typical.rust.before.p50 = 1000' <<< "$report")"
reject "$(jq '.microseconds_per_call.typical.rust.before.p50 = "100"' <<< "$report")"
reject "$(jq '.samples_per_variant = 22' <<< "$report")"
reject "$(jq '.framework_uuid = "other"' <<< "$report")"
reject "$(jq '.framework = "/other/WebKit"' <<< "$report")"
reject "$(jq '.baseline = "other"' <<< "$report")"

printf 'Agent link performance gate tests passed.\n'
