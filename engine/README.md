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

1. **Runner.** `xcode-27`: macOS 27 with Xcode 27 on Apple silicon (3 CPUs, 7 GB RAM), free for
   public repositories. WebKit main needs Xcode 26.2 or later (`checkRequiredSystemConfig` in
   `Tools/Scripts/webkitdirs.pm`) and takes its deployment target from the SDK, so the newest
   image gives frameworks for the newest macOS. `workflow_dispatch` takes another runner label
   and Xcode (`macos-26` with `Xcode_26.6.app`, for example).
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
5. **Build** ([build](webkit/scripts/build)): `Tools/Scripts/build-webkit` with the flags below,
   the full transcript in `.build/webkit-build.log` (uploaded as `webkit-macos-build-log`), a
   heartbeat on the console every five minutes, and a time budget (`QUICKIE_BUILD_MINUTES`, 300
   on CI) that stops the build and exits 124 before the 6-hour job limit. The heartbeat makes a
   run that ran out of time still say how far it got.
6. **Package** ([package](webkit/scripts/package)): `WebKitBuild/Release/*.framework`,
   `*.dylib` and any top-level `*.xpc` (the XPC services live inside `WebKit.framework`) plus
   `BUILD-INFO` into `webkit-macos-arm64-<commit12>.tar.gz`, with its `.sha256`.
7. **Attest and upload**, in the same job that built the archive: a build provenance
   attestation of the archive (`actions/attest-build-provenance`), then the artifact
   `webkit-macos-arm64-<commit12>` (archive, sha256, `BUILD-INFO`), kept 30 days.

### Build flags

None of them changes what a web page sees; each is explained next to it in
[build](webkit/scripts/build).

| Flag | Why |
|---|---|
| `--release` | Optimized, assertions off |
| `--only=Everything up to WebKit` | The frameworks and their XPC services only; the default scheme also builds MiniBrowser, WebKitTestRunner, DumpRenderTree and TestWebKitAPI |
| `--lto-mode=none` | WebKit's Release default is ThinLTO; its link does not fit 7 GB of RAM comfortably and costs time. Revisit for shipping builds |
| `GCC_GENERATE_DEBUGGING_SYMBOLS=NO`, `DEBUG_INFORMATION_FORMAT=dwarf` | No debug info and no dSYM step: less disk and time |
| `COMPILATION_CACHE_CAS_PATH=.build/webkit-cas` | Xcode's compilation cache (on in WebKit's configurations) kept where it is measured and can be handed on |

No `--no-<feature>` flags: turning features off would change web behaviour.

### Numbers

Measured on CI; see the workflow runs for the current ones.

| | |
|---|---|
| Free disk before / after cleanup | TODO(measure) |
| Sparse checkout | TODO(measure) |
| Build time | TODO(measure) |
| `WebKitBuild` / compilation cache | TODO(measure) |
| Archive | TODO(measure) |

## Bumping the pin

1. Read the new commit off the fork: `git ls-remote https://github.com/izevg/WebKit refs/heads/main`
   (or the patch branch, once there is one).
2. Put it into [webkit.pin](webkit/webkit.pin) as `WEBKIT_COMMIT`, with `WEBKIT_CHECKED` set to
   today (UTC), and the subject line in the comment.
3. Open a pull request; once it lands on `release/**` or `main`, the push builds it. To build a
   branch before that, run the workflow by hand:
   `gh workflow run webkit.yml --repo izevg/quickie --ref <branch>`.

`sync` prints whether the branch has moved past the pin, so a stale pin shows in every run.

Our patches will be commits on a branch of the fork (rebased onto WebKit main, or onto the
Safari release branch the decision names); the pin then names that branch and its head. No patch
files are kept here, and no git submodule.

## How `engine-webkit` will consume the artifact

Not wired yet; this is the plan the artifact is shaped for.

1. **Fetch and verify.** A task downloads the artifact of the pinned commit
   (`gh run download --name webkit-macos-arm64-<commit12>`), checks the sha256 and
   `gh attestation verify <archive> --repo izevg/quickie`, and unpacks it into
   `.build/webkit-frameworks/`. `BUILD-INFO` names the WebKit commit, Xcode and SDK; a mismatch
   with `webkit.pin` is an error.
2. **Development.** The shell runs against the unpacked frameworks with `DYLD_FRAMEWORK_PATH`
   (and `__XPC_DYLD_FRAMEWORK_PATH` for the XPC services), as WebKit's own `run-minibrowser`
   does. `engine-webkit` keeps using the `WKWebView` API through `objc2`; only the frameworks
   underneath change.
3. **The app bundle.** The frameworks go into `Quickie.app/Contents/Frameworks/`, signed with the
   app. The hardened runtime ignores `DYLD_*`, so the shipping build needs install names the app
   resolves itself (`@rpath`) or WebKit's staged-framework layout, as Safari Technology Preview
   uses; which one is decided with the first bundle (open question below).

## Linux (planned)

WebKitGTK from the same pin is a first-class target and comes next. The planned
`webkit-linux` job is sketched at the end of [webkit.yml](../.github/workflows/webkit.yml):
`ubuntu-24.04` and `ubuntu-24.04-arm`, the same `sync`, dependencies from WebKit's SDK container
(`wkdev-sdk`, pinned by digest), `build-webkit --gtk --release` without docs, introspection,
MiniBrowser and API tests, `ninja install` into a staging prefix, then the same tar, sha256 and
attestation. `sync` is already portable; `build` and `package` grow a GTK branch then.

## Open questions

1. Can the free runner finish a Release build inside one 6-hour job? If not: chained jobs that
   hand the build tree and the compilation cache to the next job, a larger runner, or splitting
   the build by project (JavaScriptCore and WTF first, then WebCore, then WebKit).
2. Install names for the app bundle: `@rpath` build settings or WebKit's staged frameworks.
3. ThinLTO for shipping builds once the build fits; it changes speed, not behaviour.
4. Which Safari release branch the fork follows for macOS and which WebKitGTK stable branch for
   Linux, so one patch series applies to both (the decision's "one pinned WebKit revision").
