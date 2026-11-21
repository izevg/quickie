# Engine

Quickie's engine is our fork of WebKit, patched and built by us
([architecture.md](../docs/architecture.md), "Decision: our own WebKit"). Until the fork ships,
both profiles run on the system `WKWebView` through `engine-webkit`. The Chromium scaffold is
parked under [chromium/](chromium/README.md) as reference.

This page lives in `engine/` rather than `docs/` because `docs/` already holds its four files
(the [rule of four](../AGENTS.md#the-rule-of-four)).

| Path | What |
|---|---|
| [webkit/webkit.pin](webkit/webkit.pin) | The fork, the branch, the commit that is built, the date it was checked |
| [webkit/PATCHES.md](webkit/PATCHES.md) | The patch plan: what each fork patch is for, where it goes, its tests, the order |
| [webkit/scripts/sync](webkit/scripts/sync) | Fetches exactly the pinned commit into `.build/webkit` |
| [webkit/scripts/build](webkit/scripts/build) | Builds the macOS Release frameworks with `build-webkit` |
| [webkit/scripts/package](webkit/scripts/package) | Packs them with `BUILD-INFO` and a sha256 |
| [../.github/workflows/webkit.yml](../.github/workflows/webkit.yml) | Runs the three on CI, attests and uploads the result |
| [chromium/](chromium/README.md) | The parked Chromium scaffold |

## Where it is built: CI only

The engine is built on GitHub-hosted runners, never on a developer's machine: a WebKit checkout
plus its build output is tens of gigabytes and hours of a laptop's CPU. `sync` and `build` refuse
to run outside CI unless `QUICKIE_WEBKIT_LOCAL=1` is set.

The `webkit` workflow runs on `workflow_dispatch` and on pushes to `main` and `release/**` that
touch `engine/webkit/`, the workflow, or its disk cleanup script. One job, `webkit-macos`:

1. **Runner.** `macos-26` with `Xcode_26.6.app`, free for public repositories. The fork follows
   the `webkitglib/2.54` series, whose Cocoa code does not build against the macOS 27 SDK
   (`NSRefreshControlHosting` is gone from AppKit 27); frameworks built with SDK 26 run on
   macOS 26 and 27. `workflow_dispatch` takes another runner label and Xcode (`xcode-27` with
   `Xcode_27.2.app`, for example); the default moves to Xcode 27 with a series that builds there.
2. **Free disk.** [free-disk-macos.sh](../.github/scripts/free-disk-macos.sh) deletes every Xcode
   but the selected one, the simulator runtimes, the Android SDK, .NET, GHCup, CodeQL and the
   browsers, printing the free space after each step.
3. **Metal toolchain.** `xcodebuild -downloadComponent MetalToolchain`; Xcode 26 and later ship
   without it, and WebCore and WebGPU compile shaders.
4. **Sync** ([sync](webkit/scripts/sync)): `git fetch --depth 1 --filter=blob:none` of the pinned
   commit by hash, then a sparse checkout of everything except `LayoutTests`, `JSTests`,
   `PerformanceTests`, `ManualTests`, `WebDriverTests` and `Websites`. `Source/`, `Tools/`,
   `Configurations/`, `WebKitLibraries/`, `WebKit.xcworkspace` and the root CMake files stay:
   `build-webkit` reads them for both the Xcode (macOS) and the CMake (GTK) build. Re-running
   at the same pin only re-applies the sparse patterns. It checks the free disk first
   (`QUICKIE_MIN_FREE_GB`, default 15) and prints the commit, whether the branch has moved on,
   the checkout's size and the disk left.
5. **Compilation cache.** Xcode's compilation cache (`.build/webkit-cas`, ~5 GB) is restored
   from the newest earlier run on the same runner and Xcode, and saved after the build, even
   one that failed or ran out of time, so a rebuild is mostly cache hits. `WebKitBuild` (~9 GB)
   is not cached: both would not fit the repository's 10 GB of Actions caches. GitHub scopes
   caches by ref: a run restores only what runs on its own branch or on `main` saved (a pull
   request's run also its base branch's). Build patch branches by dispatching on
   `release/0.1.0` (below), so they share the cache its pushes save; a run dispatched on a
   feature branch starts cold.
6. **Build** ([build](webkit/scripts/build)): `Tools/Scripts/build-webkit` with the flags below,
   the full transcript in `.build/webkit-build.log` (uploaded as `webkit-macos-build-log`), a
   heartbeat on the console every five minutes, and a time budget (`QUICKIE_BUILD_MINUTES`, 300
   on CI) that stops the build and exits 124 before the 6-hour job limit. The heartbeat makes a
   run that ran out of time still say how far it got.
7. **Package** ([package](webkit/scripts/package)): `WebKitBuild/Release/*.framework`,
   `*.dylib` and `*.xpc` plus `BUILD-INFO` into `webkit-macos-arm64-<commit12>.tar.gz`, with its
   `.sha256`. The XPC services sit next to the frameworks;
   `WebKit.framework/Versions/A/XPCServices/*.xpc` are relative links to them, so the archive is
   unpacked as one directory and never split.
8. **Attest and upload**, in the same job that built the archive: a build provenance
   attestation of the archive (`actions/attest-build-provenance`), then the artifact
   `webkit-macos-arm64-<commit12>` (archive, sha256, `BUILD-INFO`), kept 30 days.

### Build flags

None of them changes what a web page sees; each is explained next to it in
[build](webkit/scripts/build).

| Flag | Why |
|---|---|
| `--release` | Optimized, assertions off |
| `--only=Everything up to WebKit` | The frameworks and their XPC services only; the default scheme also builds MiniBrowser, WebKitTestRunner, DumpRenderTree and TestWebKitAPI |
| `--lto-mode=none` | WebKit's Release default is ThinLTO, whose links are the heaviest steps in memory and time on a 7 GB runner (not measured yet). Revisit for shipping builds |
| `GCC_GENERATE_DEBUGGING_SYMBOLS=NO`, `DEBUG_INFORMATION_FORMAT=dwarf` | No debug info and no dSYM step: less disk and time |
| `COMPILATION_CACHE_CAS_PATH=.build/webkit-cas` | Xcode's compilation cache (on in WebKit's configurations) kept where it is measured and can be handed on |

No `--no-<feature>` flags: turning features off would change web behaviour.

### Numbers

The first green build, at `a6331649` on 2026-11-20
([run 44666598719](https://github.com/izevg/quickie/actions/runs/44666598719)), on `xcode-27`
(macOS 27.0.1, Xcode 27.2, SDK 27.2, 3 CPUs, 7 GB RAM):

| | |
|---|---|
| Free disk before / after cleanup | 36.8 GB / 128.1 GB (180 GB volume): Xcodes +6.8, simulators +58.9, the rest +25.6; 1 min 40 s |
| Sparse checkout | 1.9 GB, 1 min 24 s |
| Build | 2 h 17 min, 5,499 objects, peak swap 0.45 GB, 17 GB of disk used |
| `WebKitBuild` / compilation cache | 9.0 GB / 4.8 GB |
| Archive | 77.6 MB (291 MB unpacked: WebCore 108 MB, WebKit 62 MB, JavaScriptCore 45 MB, libwebrtc 14 MB) |
| Whole job | 2 h 21 min of the 6 h limit |

The archive also carries `libWebCoreTestSupport.dylib`, which `Everything up to WebKit` builds;
the app bundle leaves it out.

## Bumping the pin

1. Read the new commit off the fork:
   `git ls-remote https://github.com/izevg/WebKit refs/heads/quickie/2.54`.
2. Put it into [webkit.pin](webkit/webkit.pin) as `WEBKIT_COMMIT`, with `WEBKIT_CHECKED` set to
   today (UTC), and the subject line in the comment.
3. Open a pull request; once it lands on `release/**` or `main`, the push builds it. To build a
   branch before that, run the workflow by hand:
   `gh workflow run webkit.yml --repo izevg/quickie --ref <branch>`.

`sync` prints whether the branch has moved past the pin, so a stale pin shows in every run.

Our patches are commits in the fork: each on its own branch `quickie/patch/<area>-<name>`, built
there first (below), then merged into `quickie/2.54`, whose new head the pin then names. No patch
files are kept here, and no git submodule. What each patch is for and in which order they come:
[webkit/PATCHES.md](webkit/PATCHES.md).

## Building a patch branch

A patch branch of the fork is built before it merges into `quickie/2.54`, without touching the
pin: the `webkit` workflow's `commit` input (a full 40-character fork commit) replaces
`WEBKIT_COMMIT` for that run.

```sh
commit=$(git ls-remote https://github.com/izevg/WebKit refs/heads/quickie/patch/<area>-<name> | cut -f1)
gh workflow run webkit.yml --repo izevg/quickie --ref release/0.1.0 -f commit="$commit"
```

- `sync` reads the override from `QUICKIE_WEBKIT_COMMIT` (the workflow sets it from the input),
  checks it like the pin (40 lowercase hex characters) and says it is building an override.
- `BUILD-INFO` records the commit that was built (`webkit_commit`) next to the pinned one
  (`webkit_pinned`), and the artifact is named after the built commit
  (`webkit-macos-arm64-<commit12>`), so a patch build is never mistaken for the pin's.
- Runs for different commits do not cancel each other: the concurrency group holds the commit.
  A second run for the same ref and commit still cancels the first.
- The compilation cache is shared: its key prefix stays per runner and Xcode, so a patch build
  restores the newest cache saved on that runner and Xcode (usually a build of the base) and
  recompiles little more than what the patch touches.

## How the app consumes the artifact

1. **Fetch and verify.** `engine/webkit/scripts/fetch` (`mise run webkit-fetch`) takes the
   archive of the pinned commit from the `webkit-<commit12>` prerelease, where `webkit.yml` keeps
   every build of a pushed branch (artifacts expire after 30 days; a commit kept there is not built
   again, see the `needed` job), else from the newest
   unexpired artifact. It checks the sha256, `gh attestation verify <archive> --repo
   izevg/quickie`, and that `BUILD-INFO` names the pin, then unpacks it into
   `.build/webkit-frameworks/`.
2. **The app bundle.** `cargo xtask bundle --webkit .build/webkit-frameworks`
   (`tools/xtask/src/bundle/webkit.rs`) copies the frameworks, the libraries, and the helper
   processes into `Quickie.app/Contents/Frameworks/` (the `.xpc` services move into
   `WebKit.framework/Versions/A/XPCServices`), rewrites the system install names
   (`/System/Library/...`) to `@rpath/...`, gives every binary the `LC_RPATH` to
   `Contents/Frameworks`, and signs from the inside out, each helper with its own entitlements
   (`allow-jit` for the content process) minus `get-task-allow`. The app's binary also carries
   `DYLD_FRAMEWORK_PATH` and `DYLD_LIBRARY_PATH` in its own `LC_DYLD_ENVIRONMENT`
   (`crates/shell/build.rs`): system frameworks the app loads deep down link `WebKit` by its
   absolute path, and without it both copies would load into one process. dyld honors it under
   the hardened runtime, as it is part of the signed binary. Checked: the app and every helper
   map only the bundle's `WebKit`, `WebCore`, and `JavaScriptCore`. `mise run fork` does both
   steps for a debug app and runs it.
3. **Tests.** `mise run conformance-fork` runs the engine conformance suite with
   `DYLD_FRAMEWORK_PATH` (and `__XPC_DYLD_FRAMEWORK_PATH` for the helpers) on the unpacked
   build, as WebKit's own `run-minibrowser` does; CI runs it as `engine conformance (WebKit
   fork)`. `engine-webkit` uses the same `WKWebView` API through `objc2` on either engine.
4. **The minimum macOS** is the fork's deployment target (26.5 with Xcode 26.6), which the
   bundle's `LSMinimumSystemVersion` takes from the build.

## Linux (planned)

WebKitGTK from the same pin is a first-class target and comes next. The planned
`webkit-linux` job is sketched at the end of [webkit.yml](../.github/workflows/webkit.yml):
`ubuntu-24.04` and `ubuntu-24.04-arm`, the same `sync`, dependencies from WebKit's SDK container
(`wkdev-sdk`, pinned by digest), `build-webkit --gtk --release` without docs, introspection,
MiniBrowser and API tests, `ninja install` into a staging prefix, then the same tar, sha256 and
attestation. `sync` is already portable; `build` and `package` grow a GTK branch then.

## Open questions

1. Headroom: a clean build takes 2 h 17 min of the 6 h limit. If WebKit grows past the budget
   (300 min), the plan is chained jobs that hand `WebKitBuild/` and the compilation cache
   (14 GB together) to the next job as an artifact; Xcode's compilation cache turns the rebuild
   after a fresh checkout into cache hits. A larger runner is the paid alternative.
2. WebKit main's clean build races on `availability-overlay.yaml` (JavaScriptCore's
   `testLibJSCToolsHelper` does not depend on bmalloc, which writes it); [build](webkit/scripts/build)
   writes the file first. Fix upstream, or in the fork, and drop the workaround.
3. Install names: the bundle rewrites them after the build (`install_name_tool`); building the
   fork with `@rpath` install names would drop that step.
4. ThinLTO for shipping builds: with 3.5 h of headroom it may fit; measure its time and memory
   first. It changes speed, not behaviour.
5. Which Safari release branch the fork follows for macOS and which WebKitGTK stable branch for
   Linux, so one patch series applies to both (the decision's "one pinned WebKit revision").
