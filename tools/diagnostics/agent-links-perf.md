# Agent link discovery benchmark

Run the release budgets on a quiet macOS machine with the pinned fork available. By default
the task uses `.build/webkit-frameworks`, populated by `mise run webkit-fetch`. An existing
local build can supply its framework directory.

```sh
QUICKIE_WEBKIT_FRAMEWORKS=/path/to/WebKitBuild/Release mise run performance-fork
```

For a raw JSON report with a custom sample count, call the benchmark directly using the
repository's pinned tools.

```sh
mise exec -- bash tools/diagnostics/agent-links-perf.sh /path/to/WebKitBuild/Release 32
```

The benchmark checks which WebKit framework the test loaded. It uses an offscreen `WKWebView`,
one loopback fixture per workload, and the same framework for both variants. The reference
collector and parser are from Quickie commit `48501264252595d7d98a0aec62da60c1f030bbf9`.
The conformance binary compares exact collector output, compares the parsed link snapshots,
and checks the result against the production engine port.

The `typical` fixture uses 160 shallow links. The `deep` fixture uses 1,024 links under
36 shared ancestors. Each pair alternates which implementation runs first. The output
reports the median and 95th percentile in microseconds per call after four warm-up pairs.
Each WebKit sample batches 32 calls and times the JavaScript in the isolated content world.
Each Rust sample batches 128 parses of the same reply. WebKit load and IPC are outside the
timed stages. Keep other builds, playback, and benchmark processes idle during comparisons.

## Release limits

The default gate measures 24 sample pairs and checks each median against the median of the
reference from the same run. A ratio below 1 means the candidate is faster. The gate requires
valid timings for both workloads and both stages, a verified WebKit image, and the expected
reference and sampling protocol. It reports p95 for diagnosing tail behavior.

| Workload | Stage | Maximum candidate/reference median |
|---|---|---:|
| Typical | WebKit collector | 1.08 |
| Deep | WebKit collector | 0.97 |
| Typical and deep | Rust parser | 0.96 |

Two initial runs on an M1 using the locally built fork measured ratios around 0.966 for the typical
collector, 0.826–0.844 for the deep collector, and 0.764–0.775 for the parser across both fixtures.
The shallow fixture has less repeated ancestor work to remove and its WebKit budget allows an
8% slowdown against the reference. The deep and Rust budgets retain a measurable improvement
while allowing for hosted-runner variation. The GitHub job logs keep the full JSON, including
the framework UUID and host metadata, so later runs can inform threshold calibration and a
future p95 budget.
