# Engine: Chromium for Quickie

> Status: P1 scaffold (2026-11-19). **Never compiled.** It was written without a Chromium
> checkout, against Chromium sources read on chromium.googlesource.com at the pinned tag. Every
> assumption that only a real build can confirm is marked `TODO(verify)` in the code and listed
> below. The first build on a real machine is expected to fail in a few of those places.

This page lives in `engine/` rather than `docs/` because `docs/` already holds its four files
(the [rule of four](../AGENTS.md#the-rule-of-four)).

## What gets built

Chromium Extended Stable, with the ungoogled-chromium patches, pruning and domain substitution
([privacy.md](../docs/privacy.md)), plus ours: two small patches, one file override, and
`//quickie`, the C++ and Objective-C++ glue that hands the window to the Rust shell
([architecture.md](../docs/architecture.md), sections 3 and 4.4).

| Pin ([chromium.pin](chromium.pin)) | Value | Source, checked 2026-11-19 |
|---|---|---|
| Chromium | 156.0.8078.109 (`f8ec2e9b`) | Chromium Dash, Extended channel, Mac; released 2026-11-17 |
| ungoogled-chromium | 156.0.8078.79-1 | GitHub tags; no tag for .109 exists, the M156 series is applied to it |
| ungoogled-chromium-macos | 156.0.8078.79-1.1 | GitHub tags; only `fix-disabling-safebrowsing.patch` is applied |
| depot_tools | `e0ac4551` | `main` on 2026-11-19; self-update is off (`DEPOT_TOOLS_UPDATE=0`) |
| cxx | 1.0.202 | The copy Chromium vendors at the pin; `Cargo.toml` pins the same |

## Machine

Apple Silicon, **at least 64 GB of RAM and 150 GB free** on an APFS volume, a checkout path
without spaces, current Xcode with the macOS SDK, `python3` (depot_tools brings its own), and
[mise](https://mise.jdx.dev) for the repository's own gates. Everything lands in `.build/`
(ignored): about 40 GB for the no-history checkout and 60-100 GB for the output.

## Fetch, apply, build

```sh
engine/scripts/sync     # depot_tools, Chromium at the tag (no history), ungoogled series
engine/scripts/apply    # pruning, ungoogled patches, domain substitution, ours, overrides, //quickie
engine/scripts/build    # args.gn, gn gen, gn check //quickie/*, autoninja chrome
open .build/chromium/src/out/Quickie/Chromium.app
```

Each script is idempotent and refuses to start with less free disk than `QUICKIE_MIN_FREE_GB`
(default 25). `apply` stamps the tree; editing `engine/quickie/`, `engine/overrides/` or `crates/`
needs only another `apply` (links and copies) and `build`. Changing the pin or `engine/patches/`
needs `engine/scripts/sync --reset` first, which discards everything `apply` did.

| Variable | Default | Used by |
|---|---|---|
| `QUICKIE_BUILD_DIR` | `<repo>/.build` | all |
| `QUICKIE_MIN_FREE_GB` | `25` | all |
| `QUICKIE_MACOS_PATCHES` | `ungoogled-chromium/macos/fix-disabling-safebrowsing.patch` | apply |
| `PATCH_BIN` | `patch` on `PATH` | apply (ungoogled's `patches.py`) |
| `QUICKIE_OUT` | `out/Quickie` | build |
| `QUICKIE_JOBS` | one per 2 GB of RAM, at most one per CPU | build |

## Layout

| Path | What |
|---|---|
| [chromium.pin](chromium.pin), [gclient.template](gclient.template) | What is fetched |
| [scripts/](scripts/) | `sync`, `apply`, `build`, and `args.gn` (written after ungoogled's `flags.gn`, so ours win) |
| [patches/](patches/) | Our patch series, applied after ungoogled's: `startup/link-quickie.patch` links `//quickie` into Chrome; `startup/browser-main-parts.patch` adds Quickie's `ChromeBrowserMainExtraParts` |
| [overrides/](overrides/) | Whole files copied over the tree: the `BrowserWindow` factory, with a hook asked first |
| [quickie/](quickie/) | `//quickie` (linked as `src/quickie`): `main_parts`, `bridge/`, `browser_window/`, `keyboard/` |
| [../crates/adapters/engine-chromium](../crates/adapters/engine-chromium) | The Rust side of the bridge (linked as `src/quickie_crates`) |

### How the pieces meet

- **Startup.** `MainParts` (`quickie/main_parts.cc`) adds `--no-startup-window`, remembers the
  initial profile, and calls `ShellStart()` in Rust after browser start.
- **Tabs.** `bridge/quickie_bridge.h` is the C++ side of the cxx bridge, keyed by the domain's
  `u64` tab id: create, show, navigate, close, adopt, destroy a jar. Events go back to Rust:
  navigation committed, title changed, page opened a tab. `bridge/tab_registry.*` maps tab ids to
  WebContents and keeps one `Browser` per (shell window, profile), so Chromium's `TabStripModel`
  stays the truth about live pages (architecture section 4.3) and `chrome.tabs` sees them.
- **The port.** `ChromiumEngine` in the Rust crate implements the application's `EnginePort` on
  the bridge. It keeps what the C++ side leaves to Rust: which tabs have a page, the tab a window
  is to show before the shell attached its host view (shown on `attach_host`), and which
  throwaway jars got a profile (`destroy_jar` of a jar whose tabs were never shown calls nothing).
  Events are recorded, never handled inside a bridge call: Rust asks `ScheduleWake` to post
  `RunWake`, and the shell drains `take_navigations` and `take_opened` from that task, like the
  `WebKit` engine's waker.
- **Window.** The factory override asks `quickie::MaybeCreateBrowserWindow` first. Browsers the
  registry creates, and normal or popup Browsers Chromium creates on its own, get a
  `QuickieBrowserWindow`: no Views, the active page's `NSView` added under the shell's host view.
  DevTools, apps and picture-in-picture keep Chromium's windows.
- **Keys.** `QuickieBrowserWindow::PreHandleKeyboardEvent` offers each key-down to Rust before
  the renderer (reserved shortcuts), `HandleKeyboardEvent` once the page declined it.
- **Throwaway jars.** One off-the-record profile per jar, made with
  `OTRProfileID::CreateUniqueForDevTools()`: Chromium lets only primary, DevTools and Media
  Router OTR profiles host `Browser` windows (`chrome/browser/profiles/profile.cc`), and this is
  how CDP's `Target.createBrowserContext` makes its isolated contexts. Storage is in memory and
  goes with `ProfileDestroyer::DestroyOTRProfileWhenAppropriate`.
- **Linking.** Both hooks into Chromium code (`CreateBrowserMainExtraParts`,
  `MaybeCreateBrowserWindow`) have weak defaults on Chromium's side, so test targets that do not
  link `//quickie` still link. The Rust crate builds under GN with the `chromium` feature; Cargo
  builds it without, where every engine call fails as unavailable and nothing references a C++
  symbol (tests never call FFI; `ChromiumEngine` is tested against a fake bridge).

## Verified at the pin

Read in the 156.0.8078.109 sources: the `ChromeBrowserMainExtraParts` hooks and where
`ChromeBrowserMainParts::Create` adds parts; the factory body copied into the override;
`BrowserWindowCreateParams::window` is test-only (`CHECK_IS_TEST`), hence the factory hook; every
pure virtual of `BrowserWindow` and `ui::BaseWindow`, cross-checked with `WebUIBrowserWindow`;
`TabStripModel` insert, detach, activate and close; `TabStripModelChange::Insert`;
`WebContents::Create`, `GetNativeView().GetNativeNSView()`, `UpdateWebContentsVisibility`;
`NavigationController` load, back, forward, reload; the `WebContentsObserver` hooks; the OTR
profile rules and `ProfileDestroyer`; `NativeWebKeyboardEvent::os_event` as `OwnedNSEvent::Get()`;
the `rust_static_library` parameters (`cxx_bindings`, `allow_unsafe`, `features`, `edition`) and
the generated header path; the cxx include path `third_party/rust/cxx/v1/cxx.h`; the GN args in
`args.gn` that exist at the pin; both patches apply (`patch --dry-run`) to the pinned files, and
no ungoogled patch touches their hunks. The cxx-generated C++ for `ffi.rs` compiled against
`quickie_bridge.h` (`clang++ -fsyntax-only` with cxx 1.0.202's header) before `ScheduleWake` and
`RunWake` were added; that check is not repeated yet (see below).

## TODO(verify)

| Where | What only a build or a run can tell |
|---|---|
| [scripts/sync](scripts/sync) | gclient `--no-history` with a solution pinned to `refs/tags/...` |
| [scripts/apply](scripts/apply) | Whether more of the ungoogled macOS series is needed with depot_tools toolchains |
| [scripts/apply](scripts/apply) | Domain substitution on a git checkout (files listed but absent) |
| [scripts/apply](scripts/apply) | Apple's `patch` vs GNU patch for ungoogled's series (`PATCH_BIN`) |
| [scripts/apply](scripts/apply) | Siso remote execution and the symlinks out of the checkout (local builds follow them) |
| [scripts/args.gn](scripts/args.gn) | Official build with `symbol_level = 0` (dSYM steps) |
| [patches/startup/browser-main-parts.patch](patches/startup/browser-main-parts.patch), [overrides/](overrides/) | ld64 resolves the weak defaults to the strong definitions in `//quickie` |
| [quickie/BUILD.gn](quickie/BUILD.gn) | Dependency labels (`gn check`); `crate_root` outside the target's directory; GN targets for the crate's new dependencies (`quickie-application`, `quickie-domain-browsing`, `url`) |
| [quickie/bridge/quickie_bridge.cc](quickie/bridge/quickie_bridge.cc) | `ScheduleWake`: `base::BindOnce(&RunWake)` on the cxx-generated `extern "Rust"` function; the generated `ffi.rs.h` still passes `clang++ -fsyntax-only` with `ScheduleWake` and `RunWake` (`base/task/sequenced_task_runner.h`) |
| [../crates/adapters/engine-chromium/src/engine/reports.rs](../crates/adapters/engine-chromium/src/engine/reports.rs) | A commit keeps the last title until the page reports one: that Chromium calls `TitleWasSet` after every cross-document commit, even without a `<title>` (`content/browser/web_contents/web_contents_impl.cc`) |
| [quickie/bridge/tab_registry.h](quickie/bridge/tab_registry.h) | DevTools-prefixed OTR profiles as jars: other `IsDevTools()` effects; extensions off there (`chrome/browser/profiles/profile.cc`) |
| [quickie/bridge/tab_registry.cc](quickie/bridge/tab_registry.cc) | A Browser Chromium creates may get its first tab before the factory returns (covered by a posted scan); pages Chromium destroys are not reported to Rust yet |
| [quickie/main_parts.cc](quickie/main_parts.cc) | Dock-icon reopen (`chrome/browser/app_controller_mac.mm`) still creates a Browser |
| [quickie/browser_window/quickie_browser_window.mm](quickie/browser_window/quickie_browser_window.mm) | `Close()` leads to the Browser's deletion without a `views::Widget` |
| same | `GetWebContentsModalDialogHost()` is nullptr: JavaScript `alert`, HTTP auth (`components/web_modal`) |
| same | `GetExclusiveAccessContext()` is nullptr: fullscreen, pointer lock (`chrome/browser/ui/exclusive_access`) |
| same | `GetLocationBar()`, `CreateFindBar()`, `GetAutofillBubbleHandler()` are nullptr; the intent picker drops its callback |
| same | Unhandled key equivalents no longer reach Chromium's main menu (`browser_view.cc`, `HandleKeyboardEvent`) |
| same | The page view goes below the shell's overlay views in the host view |

## Open questions for the first build

1. Does `gclient sync --no-history` at the tag, then `apply`, leave a tree that `gn gen` accepts
   (pruning with `--keep-contingent-paths`, domain substitution on a git checkout)?
2. Which `gn check` errors does `//quickie` have? Expect missing dependency labels.
3. Does the M156 ungoogled series (cut for .79) apply cleanly to .109?
4. Does `alert()` crash with a null modal dialog host? If so, a minimal
   `WebContentsModalDialogHost` on the host view is the first follow-up.
5. Does a Browser actually close and get deleted when its last tab moves to another jar's
   Browser, and does `DestroyJar` then free the OTR profile?
6. Starting the shell: `ShellStart()` in the Rust bridge is a no-op until the composition root
   (`crates/shell`) exists. The plan is an `extern "Rust"` symbol the shell crate defines and this
   crate declares, so the adapter never depends on the shell; GN then links the shell crate into
   `//quickie`.
7. Rust crates beyond `cxx`: the adapter now depends on `quickie-application`, so every workspace
   crate it reaches needs a GN `rust_static_library`, and third-party crates go through `gnrt`
   into `.build/` (architecture section 1).
8. Pages closed by Chromium itself (`window.close()`, a crashed renderer) need an engine event so
   the domain moves the tab to the trash instead of losing track of it.
9. Opened pages: `EnginePort::adopt` binds the engine's page handle to the new tab before
   `Browsing::adopt` shows it (resolved). Still open: pages Chromium opened and Rust never adopts
   have no bridge call to close them; they go only with their jar or the Browser.

## Rebasing to a new Extended Stable

1. Update [chromium.pin](chromium.pin) (Chromium Dash, ungoogled tags), and `cxx` in
   `Cargo.toml` to the copy vendored at the new tag.
2. `engine/scripts/sync --reset && engine/scripts/apply`. A failing hunk in
   [patches/](patches/) is regenerated against the new file; [overrides/](overrides/) are diffed
   against the new upstream file and the hook carried over (search for "Quickie").
3. Re-check the `BrowserWindow` pure virtuals against `chrome/browser/ui/browser_window.h`.
