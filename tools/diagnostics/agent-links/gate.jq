def nonempty: type == "string" and length > 0;
def positive: type == "number" and isfinite and . > 0;
def timing: (.p50 | positive) and (.p95 | positive) and .p95 >= .p50;

def limit($workload; $stage):
    {typical: {webkit: 1.08, rust: 0.96}, deep: {webkit: 0.97, rust: 0.96}}[$workload][$stage];

if length != 1 then error("expected one Agent link performance report") else .[0] end
| if .benchmark != "agent-links" or .baseline != "48501264252595d7d98a0aec62da60c1f030bbf9" then
    error("unexpected Agent link benchmark or reference revision")
elif .framework != $framework or .framework_uuid != $uuid or ($uuid | nonempty | not)
        or .architecture != "aarch64" or (.cpu | nonempty | not) or (.macos | nonempty | not) then
    error("unexpected WebKit framework or missing macOS runner metadata")
elif .samples_per_variant != $samples or $samples < 20 or $samples > 80 or $samples % 2 != 0
        or .warmup_pairs != 4 or .webkit_iterations_per_sample != 32 or .rust_iterations_per_sample != 128 then
    error("unexpected Agent link benchmark sampling protocol")
else . end
| . as $report
| [
    ("typical", "deep") as $workload
    | ("webkit", "rust") as $stage
    | $report.microseconds_per_call[$workload][$stage] as $timings
    | if ($timings.before | timing) and ($timings.after | timing) then
        ($timings.after.p50 / $timings.before.p50) as $ratio
        | if ($ratio | positive) then
            limit($workload; $stage) as $limit
            | {workload: $workload, stage: $stage, before: $timings.before, after: $timings.after,
                ratio: $ratio, limit: $limit, passed: ($ratio <= $limit)}
          else error("invalid median ratio for \($workload)/\($stage)")
          end
      else error("invalid timing for \($workload)/\($stage)")
      end
] as $measurements
| $report
| del(.microseconds_per_call)
| . + {measurements: $measurements, passed: all($measurements[]; .passed)}
