# Architecture

> Status: v0.3 (2026-11-19). Languages: **C++ where it is Chromium; Rust everywhere else.**
> Items marked **[verify]** rest on an unverified source or on inference.

## 0. Principles

1. **Content first.** In the idle state only the page is on screen. UI appears only on an explicit
   key, gesture, or click. Hover-reveal is forbidden.
2. **Native.** AppKit and Core Animation through `objc2`, 120 Hz, the main thread only renders and
   applies cheap mutations.
3. **One build, one repository.** GN builds the product; Cargo builds the shell without Chromium
   for development (P0).
4. **One source of truth per entity**, with explicit engine ↔ domain sync rules (§4.3).
5. **Minimal diff against Chromium**: cheap rebases, a small patch gap.
6. **Trackerless by default, provably** ([privacy.md](privacy.md)).
7. **Never lose a tab**: transactional session mutations, deletion only through the trash.

## 1. Stack

| Layer | Technology | Why |
|---|---|---|
| Engine | Chromium Extended Stable + ungoogled-chromium patches + ours | CDP, DevTools, MV3, compatibility |
| Build | `gn` + `siso` (`autoninja`); Rust in-tree via `rust_static_library`; optional EngFlow remote execution | Rust is first-class in the Chromium tree |
| Chromium boundary | `cxx` (C++ ↔ Rust) plus thin `.mm` files where Chromium's macOS code needs Objective-C++ | No intermediate Objective-C API |
| Shell | Rust 2024, `objc2-app-kit`, `objc2-quartz-core`, `objc2-foundation`, `block2`, `dispatch2` | The same AppKit/CA that Swift would call |
| Liquid Glass | `NSGlassEffectView`/`NSGlassEffectContainerView` (macOS 26), `NSVisualEffectView` fallback (macOS 15) | Through bindings, or `msg_send!` if `objc2-app-kit` lacks the class **[verify]** |
| Shell storage | SQLite (`rusqlite`, linked against the SQLite in the Chromium tree **[verify]**), WAL, FTS5 | Sessions, MRU, palette history, keymap, agent audit |
| Palette | `nucleo-matcher` (fuzzy matcher from Helix, without `nucleo`'s thread pool) + FTS5 + frecency | Fast incremental search |
| Blocker | `adblock` crate (adblock-rust, MPL-2.0) in the network service | Network + cosmetic + scriptlets, no MV2 |
| Built-in agent | Own client for the **OpenAI-compatible API** (`/v1/chat/completions`, SSE, tool calling); HTTP through Chromium's network service | Any endpoint: hosted OpenAI-compatible APIs, Ollama (`127.0.0.1:11434/v1`), LM Studio |
| External agents | MCP via `rmcp` (official Rust SDK) on a Unix socket; CDP only as an internal channel | No TCP port |
| Secrets | Keychain via `security-framework` | LLM API keys never sit in files |
| Updates | Sparkle 2 (Objective-C, called through `objc2`), EdDSA, deltas; Developer ID + notarization | Self-distribution |
| Tests | `cargo test` for pure crates, GN `rust_unit_test`, Chromium `browser_tests` for patches, egress test | See [quality.md](quality.md) |

### Required crates

| Crate | Where | Why |
|---|---|---|
| `objc2`, `objc2-foundation`, `objc2-app-kit`, `objc2-quartz-core`, `block2`, `dispatch2` | ui, adapters | AppKit, Core Animation, GCD |
| `objc2-web-kit` | adapter `engine-webkit` | Engine stand-in for P0 and development |
| `cxx` | adapter `engine-chromium` | Chromium boundary |
| `rusqlite` (+ `fts5`) | adapter `store` | Persistence |
| `nucleo-matcher` | domain `palette` | Fuzzy search |
| `adblock` | engine | Shields |
| `rmcp`, `tokio` (one agent-IO thread only) | adapter `mcp` | MCP server on a Unix socket |
| `serde`, `serde_json`, `thiserror`, `url` | everywhere | |
| `tracing` + a local subscriber | everywhere | Logs go to disk only |
| `security-framework` | adapter `system` | Keychain |
| `indexmap` | domain | MRU stack (ordered set) |

Explicitly **not used**: Swift or SwiftUI, Apple Foundation Models (LLMs only through an
OpenAI-compatible endpoint), a second network stack (`reqwest`/`hyper` for internet traffic: all
traffic goes through Chromium so the netlog and the egress test see it), cloud sync, HTML shells.

Third-party crates are vendored into the Chromium checkout with `gnrt` (Chromium's tool), at
pinned versions, under `.build/` (not tracked).

## 2. C1 — Context

```mermaid
flowchart LR
  U((User)) -->|keyboard, mouse, trackpad| Q[Quickie.app]
  Q <-->|HTTPS| W[Websites]
  Q <-->|OpenAI-compatible API, on request only| L[LLM endpoint: local Ollama or hosted]
  A[External agents: Claude Code and others] <-->|MCP over stdio| Q
  Q -->|updates| S[Own update feed / quickie-services]
  Q -.->|opt-in only| G[Google: Widevine]
```

By default the only outbound connections are **the sites the user opens**. The LLM endpoint is
contacted only when the user asks the agent, and only the configured one.

## 3. C2 — Containers

```mermaid
flowchart TB
  subgraph App[Quickie.app — browser process]
    CR[Chromium browser process, C++]
    Glue[//quickie C++ and .mm glue]
    Shell[Rust: UI + application + domain + adapters]
    CR <--> Glue <-->|cxx| Shell
  end
  CR --> R[Renderer helpers]
  CR --> GPU[GPU helper]
  CR --> Net[Network service: adblock, LLM HTTP]
  Shell --> DB[(shell.sqlite)]
  CR --> P[(Chromium profiles: main, agent)]
  MCPB[quickie-mcp: Rust stdio-to-socket bridge] <-->|Unix socket 0600| Shell
  Ext[External agent] <-->|stdio| MCPB
```

- **One browser process, one main thread.** Chromium's UI thread is the macOS main thread, and the
  Rust shell runs on it too (`MainThreadMarker`). Heavy work (ranking, FTS, parsing LLM output)
  runs on the Chromium thread pool (through `cxx`) or on own workers; results return through the
  `dispatch2` main queue.
- **Startup.** The entry point is the stock `ChromeMain`. Our `ChromeBrowserMainExtraParts` calls
  `quickie::shell_start()` through `cxx`. The `BrowserWindow::CreateBrowserWindow` factory is
  replaced through a file override, so the Rust shell creates windows.
- **`NSApplication` is `BrowserCrApplication`** (a Chromium requirement); the shell reaches `NSApp`
  through `objc2`.
- **Two profiles:** `main` (the user) and `agent` (agent actions; no cookies or logins of `main`).
- **Development mode (P0):** the same crates build with Cargo into a standalone app on the
  `engine-webkit` adapter, so UI work never waits for a Chromium build. `cargo xtask bundle`
  (`mise run bundle`) wraps it as an ad-hoc signed `Quickie.app` that LaunchServices registers as
  a browser; signing with a Developer ID and notarizing come with the product build (§7, P1).

## 4. C3 — Components

### 4.1 Layers (DDD + hexagonal + clean) as crates

```
ui (FSD) ──▶ application (use cases, port traits) ──▶ domain (pure)
                  ▲
shell (composition root) ──▶ adapters (engine-chromium | engine-webkit, store, llm, mcp, system, updater)
```

The dependency rule points inward and is enforced by the build and by `cargo xtask layers`
([quality.md](quality.md)). Domain crates use only `std`, `thiserror`, `indexmap`,
`nucleo-matcher`, `serde`, `url`: no `objc2`, `cxx`, or `rusqlite`.

**Bounded contexts** (`crates/domain/<name>`, package `quickie-domain-<name>`). Contexts depend on
the kernel only; cross-context work is a use case. Eight directories is the rule-of-four limit: a
ninth context means regrouping first.

| Crate | Aggregates / value objects | Invariants |
|---|---|---|
| `kernel` | IDs (`TabId`, `WindowId`, `CommandId`, ...), `Timestamp`, `Site`, shared value types | — |
| `browsing` | `Window`, `Tab` (id, engine ref, pinned slot, last active), `MruStack`, `ClosedTabTrash` | MRU updates synchronously on activation; a closed tab stays 30 days in the trash |
| `commands` | `Command`, `Keymap`, `KeyBinding` (chord, `when`), `WhenExpr`, `KeyStroke` (key code + ASCII) | Browser-reserved shortcuts never reach the page; a binding conflict is a validation error |
| `palette` | `Query` (prefixes `>` `@` `#` `?` `!`), `Candidate`, `Frecency` | First batch within 16 ms, then streaming |
| `library` | `HistoryEntry`, `Download` | Local only |
| `shields` | `SitePolicy`, `FilterListSet`, `FingerprintPolicy` | Per-site exceptions; lists only from allowed sources |
| `agents` | `AgentSession` (built-in or external), `Mode` (Ask or Act), `ToolCall`, `CapabilityGrant`, `PendingAction`, `ProvenanceSet`, `LlmEndpoint` (base URL, model, key ref) | **Ask** only reads the current page of `main`, on an explicit request. **Act** only in the agent profile, URLs only from provenance, irreversible actions only after approval |
| `settings` | `Preferences`; `SiteZooms` with `ZoomLevel` (Safari's steps, 50 % to 300 %) | A site at 100 % is not listed; a throwaway jar's zoom is never saved |

**Ports** (`crates/application`), all traits:

| Port | Direction | Adapter |
|---|---|---|
| `Engine` bundle: `EnginePort` (tab lifecycle) + capability ports `PageSurface`, `PageContent`, `PageZoom`, `PageAsks`, `Downloads`, and for the agent profile `Automation`, `RequestGate`; one `EngineEvent` stream | driven | `engine-webkit`, `engine-cef` (agent profile), `engine-chromium` (post-MVP fork); see "Engines" below |
| `HttpPort` | driven | supplied by the engine (`Engine::Http`): Chromium or CEF network stack, `NSURLSession` on WebKit; the shell never builds one |
| `LlmPort` | driven | `llm`: OpenAI-compatible client on top of `HttpPort` |
| `SessionRepo`, `LibraryRepo`, `KeymapRepo`, `ShieldsRepo`, `AuditLog` | driven | `store` (rusqlite; saves go to a write-behind thread off the main thread) |
| `ContentBlockerPort` | driven | engine: `adblock` in the network service (product), `WKContentRuleList` (development) |
| `SecretStore`, `Entropy`, `Updater` | driven | `system` (Keychain, `SecRandom`), `updater` (Sparkle) |
| Clipboard, reveal in Finder, default browser, global hotkey, keyboard layout | — | Not ports until a use case calls them: today only the shell does, and a Linux shell writes its own glue |
| `AgentGateway` | driving | `mcp` (rmcp on a Unix socket) |
| UI intents | driving | `ui/features` call use cases |

#### Portability

Quickie ships on macOS; a Linux port (never Windows) must be an addition, not a rewrite, and must
cost nothing at run time.

- **The platform is chosen once, at the composition root.** The shell passes a plain value
  (`quickie_domain_commands::Platform`) into the use cases that need it; nothing downstream asks
  which platform it runs on, and there is no `cfg`, no dynamic dispatch, and no platform check on
  a hot path.
- **Domain, application, `store`, and `llm` are platform-free.** They take the platform as data
  where conventions differ (the keymap) and never reach for platform APIs.
- **Platform code lives in adapters, `ui`, and `shell` only.** A Linux port adds its own shell,
  UI crates, and system adapters next to the macOS ones.
- **Keys are written once.** Default bindings use `mod` for the primary modifier (`⌘` on macOS,
  Ctrl on Linux), resolved when the keymap is built, so a key stroke is always concrete and the
  lookup costs the same everywhere. Reserved strokes and conflicts follow from the resolved keys;
  keycaps print per platform (`⇧⌘P`, `Ctrl+Shift+P`).

#### Engines: ports, capabilities, profiles

Engines are swapped by changing one type alias, never by editing call sites. Decided 2026-11-20;
the audit behind it found ~45 shell call sites tied to `WebKitEngine` and four places where the
browser's own traffic bypassed the engine's network stack.

- **Ports name outcomes, not mechanisms.** `EnginePort` keeps the tab lifecycle (create, show,
  navigate, close, adopt, destroy a jar). Small capability ports sit next to it in
  `application/src/engine/`: `PageSurface` (the native view and thumbnails), `PageContent` (page
  text, link-hint targets, find), `PageZoom`, `PageAsks` (JavaScript dialogs, permissions, full
  screen, answered with a `ReplyToken`), `Downloads`. There is no "evaluate JavaScript" port: an
  isolated world is how WebKit does it, not what a use case needs.
- **One event stream.** Every engine reports through one `EngineEvent` enum, drained by the shell
  on the main thread after a waker. Nothing calls back into a use case from inside an engine
  call.
- **The engine is a bundle.** `trait Engine` names the associated types the shell embeds or
  forwards (`View`, `Picture`, `Page`), supplies the network stack (`Http`) and the content
  blocker, and declares `CAPS`, a `const` of what the UI may offer (programmatic inspector, blocked
  counts, ...). A capability a use case cannot work without is a trait bound, so a missing one is
  a compile error; a capability the UI merely degrades without is checked against `CAPS`, also at
  compile time. Nothing is decided at run time.
- **Policy lives in use cases.** Download ids, zoom on commit, automatic answers to dialogs and
  permissions, and collecting page text for the agent are application code driving the ports,
  tested against the in-memory fake; the shell only routes.
- **An engine per profile.** Tab and window ids carry their `Profile` (`Main`, `Agent`), and each
  profile has its own session, so a tab never moves between engines and ids never collide. The
  agent profile's tabs are never persisted: the agent leaves an audit log, not tabs. The shell
  meets two engines in exactly one place, a two-arm `match` on the profile; each arm is statically
  typed. No `dyn Engine`, no `enum AnyEngine`.
- **Act mode is two ports.** `Automation` (read the DOM and the accessibility tree, trusted input,
  navigate) is implemented on WebKit with scripts in an isolated world and synthetic `NSEvent`s,
  and on CEF or Chromium with the DevTools protocol, inside the process. `RequestGate` (hold a
  request, show its payload, let it go) only exists where the DevTools protocol does. The Act use
  case requires `Automation`; with `RequestGate` it approves the real request, without it it asks
  before the DOM action (form submit, a destructive button) and says that it cannot see the
  request. The shape of both follows the DevTools protocol, which CEF and the fork both speak.
- **Engine choice is a cargo feature at the composition root**: one alias per profile. CI also
  type-checks the shell against a second engine, so a call that only one engine supports breaks
  the build instead of a release.
- **One conformance suite.** `application` ships behavioral tests (feature `conformance`) that
  every engine adapter runs, with the fake as the reference: navigation, opened pages, dialogs,
  find, link hints, jars, downloads, the content blocker, and on CEF automation and renderer
  crashes. Pixels, print panels, the inspector and real focus stay with the harness.

Engines in this plan: `engine-webkit` for the main profile now; `engine-cef` (official prebuilt
CEF, no proprietary codecs) for the agent profile; an ungoogled-chromium fork after the MVP,
built on CI, which then hosts both profiles. Third-party CEF builds with codecs were audited and
rejected (provenance); codecs come from our own CI build if they are ever needed.

### 4.2 UI: Feature-Sliced Design as crates

| FSD layer (`crates/ui/...`) | Contents | Depends on |
|---|---|---|
| `app` | Window composition, overlay router (one active E2 at a time), theme, tokens | everything below |
| `screens` | `BrowserScreen`, `SettingsScreen`, `KeymapEditorScreen`, `AgentAuditScreen` | widgets and below |
| `widgets` | `CommandPalette`, `AddressOverlay`, `TabSwitcher`, `TabOverview`, `FindBar`, `LinkHintsLayer`, `ToastStack`, `LoadProgressLine`, `OriginChip`, `PermissionSheet`, `ShieldsPopover`, `DownloadsPopover`, `AgentAnswerPanel`, `AgentApprovalSheet` | features and below |
| `features` | `copy_url`, `copy_as_markdown`, `reopen_closed_tab`, `find_in_page`, `toggle_shields`, `ask_agent`, `approve_agent_action`, `record_shortcut`, `swipe_navigation`, `link_hints` | entities and below, application |
| `entities` | `TabRow`, `TabThumbnail`, `CommandRow`, `HistoryRow`, `KeycapGroup`, `AgentBadge`, `OriginLabel` | shared, domain |
| `shared` | `GlassSurface::new(Elevation)`, `ListView` (`NSTableView` with a diffing source), `Keycap`, spring presets (`CASpringAnimation`), colors, typography | `objc2` |

The full, mockup-backed list with states is the [component inventory](design/inventory.md); the
tokens are in the [design system](design/system.md).

Rules: slices of one layer never use each other; business logic lives only in domain and
application; lists are only `NSTableView`/`NSCollectionView`; custom views use `define_class!`,
and private AppKit methods are never overridden.

**Liquid Glass elevation:**

| Level | What | Material | Shadow | Content dimming |
|---|---|---|---|---|
| E0 | Web content | none | — | — |
| E1 | Progress line, toasts, `OriginChip`, `AgentBadge` | glass `.clear` | none | none |
| E2 | Palette, address, switcher, overview, find, popovers, `AgentAnswerPanel` | glass `.regular` inside `NSGlassEffectContainerView` | medium | 8–12 % |
| E3 | Permission, approval, auth sheets | glass `.regular` + tint | deep | 25–35 % |

Forbidden: glass on glass (an E3 collapses the E2 under it); glass in the content layer; two E2
overlays at once.

### 4.3 Engine ↔ domain sync

- **Chromium's `TabStripModel`** is the truth about live tabs (needed by `chrome.tabs` and
  `window.open`).
- **`browsing`** is the truth about organization and persistence (MRU, slots, trash, restore).
- Commands: UI → use case → `EnginePort`. Engine events (a site or an extension opened a tab) →
  adapter → `ReconcileTab` → domain. There are no silent reverse mutations.
- Each tab's navigation state (`SessionState`, Chromium's serialization) is stored in
  `shell.sqlite`; Chromium's own session restore is off.

### 4.4 Engine (`engine/quickie`, built as `//quickie` in the tree)

| Component | Language | Role |
|---|---|---|
| `bridge/` | C++ + `cxx` | C++ ↔ Rust boundary: create, close, restore tabs; events; snapshots |
| `browser_window/` | Objective-C++ | `BrowserWindow` without Views' `BrowserView`: the WebContents view (`RenderWidgetHostViewCocoa`) is embedded into an `NSView` the shell provides |
| `keyboard/` | Objective-C++ | Key equivalents: reserved ones go to Rust before the renderer; the rest go to the page first |
| `swipe/` | Objective-C++ | Hooks `HistorySwiper` and forwards events to Rust for the native preview |
| `shields/` | C++ + Rust | `adblock` in a `URLLoaderThrottle`, cosmetic filtering, CNAME uncloaking through the built-in resolver |
| `net_bridge/` | C++ + `cxx` | `HttpPort` on `SimpleURLLoader` with streaming (LLM SSE) |
| `privacy/` | C++ patches | On top of ungoogled-chromium ([privacy.md](privacy.md)) |
| `agent_bridge/` | C++ | Internal DevTools session (`DevToolsAgentHost`), agent profile only, no port |

**Developer tools.** On the WebKit stand-in every page sets the public `isInspectable`, so Web
Inspector attaches from right-click → Inspect Element or Safari's Develop menu; `WKWebView` has no
public call that opens it, and Quickie calls no private API, so the `quickie.devtools.*` commands
say where it is in a toast. On Chromium the bridge opens `DevTools` docked in the window
(`DevToolsWindow::OpenDevToolsWindow` with the asked panel, `ToggleDevToolsWindow` for the toggle),
in the main profile only and never on a port.

Inherited Views surfaces (permission bubbles, extension popups, autofill) stay in v1 as separate
windows anchored to a point; permissions and the fullscreen notice become native E3 sheets in P3.

## 5. C4 — Code

```rust
// crates/application/src/engine/ (signatures only; see "Engines" in §4.1)
pub trait EnginePort {
    type Page;                                                 // a page the engine opened itself
    fn adopt(&mut self, page: Self::Page, tab: TabId) -> Result<(), EngineError>;
    fn create(&mut self, tab: TabId, url: &Url, jar: Jar) -> Result<(), EngineError>;
    fn show(&mut self, window: WindowId, tab: TabId) -> Result<(), EngineError>;
    fn navigate(&mut self, tab: TabId, navigation: &Navigation) -> Result<(), EngineError>;
    fn close(&mut self, tab: TabId) -> Result<(), EngineError>;
    fn destroy_jar(&mut self, jar: JarId) -> Result<(), EngineError>;
}

pub trait Engine: EnginePort + PageSurface + PageContent + PageZoom + PageAsks + Downloads {
    type Http: HttpPort + Clone + Send + 'static;
    type Blocker: ContentBlockerPort;
    const CAPS: Capabilities;
    fn http(&self) -> Self::Http;
    fn drain(&mut self, into: &mut Vec<EngineEvent<Self::Page>>);
}

pub trait Automation { /* DOM and accessibility snapshots, trusted input, navigate */ }
pub trait RequestGate { /* hold, inspect, continue or fail a request: DevTools engines only */ }

pub trait LlmPort {
    /// OpenAI-compatible /v1/chat/completions, stream = true; blocks on a worker thread,
    /// tokens go to `on_event` as they arrive, `stop` (Stop ⌘.) ends it early.
    fn chat(&self, req: &ChatRequest<'_>, stop: &StopSignal, on_event: &mut dyn FnMut(ChatEvent))
        -> Result<Ended, LlmError>;
}
```

```rust
// crates/domain/browsing/src/mru.rs: a pure type, no allocation on the hot path
pub struct MruStack(indexmap::IndexSet<TabId>);
impl MruStack {
    pub fn touch(&mut self, id: TabId) { self.0.shift_remove(&id); self.0.insert_before(0, id); }
    pub fn nth(&self, i: usize) -> Option<TabId> { self.0.get_index(i).copied() }
}

// crates/domain/commands/src/keystroke.rs
pub struct KeyStroke { pub key_code: u16, pub ascii: char, pub mods: Mods }
// `ascii` comes from the ASCII-capable layout, so ⌘⇧C works on a Russian layout.
```

```rust
// engine/quickie/bridge/lib.rs
#[cxx::bridge(namespace = "quickie")]
mod ffi {
    extern "Rust" {
        type Shell;
        fn shell_start() -> Box<Shell>;
        fn host_view_for_window(self: &Shell, window: u64) -> usize;          // NSView*
        fn handle_key_equivalent(self: &mut Shell, key_code: u16, mods: u32, ascii: u32) -> bool;
        fn on_engine_event(self: &mut Shell, ev: &EngineEvent);
    }
    unsafe extern "C++" {
        include!("quickie/bridge/engine.h");
        fn create_tab(window: u64, url: &str, profile: u8) -> u64;
        fn close_tab(tab: u64) -> Vec<u8>;                                    // SessionState
        fn restore_tab(window: u64, state: &[u8]) -> u64;
    }
}
```

Hot-path budgets:

| Path | Budget | How |
|---|---|---|
| Ctrl+Tab → switch | ≤ 1 frame | MRU in memory; the overlay appears only after a hold > 150 ms |
| Open the palette | ≤ 16 ms to first frame | Prewarmed window; first batch from the in-memory MRU, FTS fills in |
| Swipe back | 120 Hz | Cached snapshot, animation on a `CALayer`; on `engine-webkit`, `WKWebView`'s own `allowsBackForwardNavigationGestures` does exactly this |
| Tab overview pinch | 120 Hz | Thumbnails cached per tab (drawn off the main thread when a tab is hidden); the grid follows the fingers by scrubbing a paused animation |
| First agent token | ≤ endpoint TTFT + 20 ms | SSE streams straight into `AgentAnswerPanel` |
| Cold start to first frame | ≤ 300 ms **[target]** | Lazy restore; background tabs discarded |

## 6. Repository layout

Follows the rule of four ([AGENTS.md](../AGENTS.md)).

```
quickie/
├─ AGENTS.md  CLAUDE.md  README.md  Cargo.toml  Cargo.lock
├─ docs/         architecture.md  lessons.md  privacy.md  quality.md  design/
├─ crates/
│  ├─ domain/       kernel  browsing  commands  palette  library  shields  agents  settings
│  ├─ application/  use cases + port traits
│  ├─ adapters/     engine-chromium (GN only)  engine-webkit (dev)  store  llm  mcp  system  updater
│  ├─ ui/           shared  entities  features  widgets  screens  app
│  └─ shell/        composition root; also the dev binary on engine-webkit
├─ engine/
│  ├─ README.md         fetch, apply, build; what is verified and what is not (see below)
│  ├─ chromium.pin      Chromium version (Extended Stable) + ungoogled-chromium revision
│  ├─ gclient.template
│  ├─ patches/          our patches only, grouped by meaning (series + one directory per area)
│  ├─ overrides/        Chromium file overrides instead of patches (BrowserWindow factory, ...)
│  ├─ quickie/          //quickie: bridge, browser_window, keyboard, swipe, shields, net_bridge, privacy, agent_bridge
│  └─ scripts/          sync, apply, rebase, build, sign, notarize (+ gn args)
├─ tools/
│  ├─ xtask/          structure, layer and commit-message checks; the development app bundle
│  ├─ quickie-mcp/    stdio-to-socket bridge for external agents
│  ├─ egress-test/    fresh profile + netlog + allowlist diff
│  └─ rebase-bot/     scripts and prompts for agent-assisted rebases
└─ services/          optional, self-hosted: update-feed, filter-mirror, component-proxy
```

Not tracked, under `.build/`: the Chromium checkout, the fetched ungoogled-chromium patch series,
vendored crates, build output.

How to fetch, patch and build the engine, and what the P1 scaffold has not yet confirmed on a real
build: [engine/README.md](../engine/README.md).

## 7. Infrastructure

| Node | Needed | Description |
|---|---|---|
| Build machine | yes | Apple Silicon, ≥ 64 GB RAM, ≥ 1 TB SSD (tree + out ≈ 150+ GB), APFS, a path without spaces, current Xcode/SDK |
| Remote execution | optional | EngFlow (Siso remote execution on macOS since M145) **[verify pricing]** |
| CI | yes | Self-hosted runner: the [quality.md](quality.md) gates → GN build → `browser_tests` → egress test → sign → notarize |
| Rebase bot | yes | Every 8 weeks (Extended Stable) and on every security release; an agent fixes conflicts, a human reviews |
| Update feed | yes | Static Sparkle appcast |
| quickie-services | optional | Component proxy, filter-list mirror, CRX proxy (the Helium / Brave `go-update` pattern) |
| LLM | optional | Local Ollama/LM Studio or any OpenAI-compatible endpoint; key in the Keychain |
| Crash | local | Crashpad writes dumps to disk only |
| Patch-gap metric | yes | Target ≤ 72 h from a Chromium security fix to our release |

## 8. Security

- **Spoofing with hidden chrome:** `OriginChip` on every origin change; a persistent origin chip on
  pages with password fields; site-requested fullscreen always shows a notice; a popup without a
  URL shows `about:blank` (lessons from Dia's 2025 CVEs).
- No server-side content that executes code (Arc Boosts, CVE-2024-45489).
- **Built-in agent:** page content goes to the LLM as untrusted data, separate from the user's
  instructions. Ask mode only reads, and only on an explicit `?`. Act mode runs only in the agent
  profile, with the provenance rule and approvals. Palette input without `?` never reaches an LLM.
- **External agents:** MCP only over a `0600` Unix socket, tools on an allowlist, the same Act
  rules, a full audit log in `shell.sqlite`.
- Hardened Runtime, minimal entitlements per helper (JIT only for renderer and GPU), Sparkle only
  with EdDSA.

## 9. Roadmap

| Phase | Result | Done when |
|---|---|---|
| P0 | Cargo-built app on `engine-webkit`: palette, MRU switcher, glass E0–E3, swipes, keymap | I live in it for a week as my main browser (without extensions) |
| P1 | Engine ports per the "Engines" section; `engine-cef` (prebuilt CEF) for the agent profile; Act mode on WebKit through `Automation` | Engines swap by one alias; the conformance suite passes on WebKit and CEF; the agent acts in its own profile |
| post-MVP | ungoogled-chromium fork built on CI (`//quickie`, `BrowserWindow` host, `cxx` bridge), hosting both profiles | The same UI on the fork, DevTools, MV3 |
| P2 | Shields + privacy patches + egress test in CI | A fresh profile: zero unsolicited hosts in 10 minutes |
| P3 | Native E3 sheets instead of Views bubbles, link hints, tab overview, keymap editor | — |
| P4 | Agents: built-in (OpenAI-compatible, Ask then Act), MCP for external ones, approvals, audit | `?` in the palette answers through Ollama; Claude Code drives a tab in the agent profile |
| P5 | Anti-fingerprinting, quickie-services | privacytests.org results on par with Brave **[target]** |
