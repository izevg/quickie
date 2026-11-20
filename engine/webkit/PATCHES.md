# Quickie WebKit fork: patch plan

Written 2026-11-20. Fork: `github.com/izevg/WebKit`. Base: the WebKitGTK/WPE stable branch
`webkitglib/2.54` (release tag `webkitgtk-2.54.3`, `eb74cd57`); the fork branch `quickie/2.54`
starts there, and its first commit `02fa3f47` is a Cocoa build fix. CI builds it on `macos-26`
with Xcode 26.6 ([engine/README.md](../README.md)); [webkit.pin](webkit.pin) names the commit
that is built.

The research below was done on WebKit main at `a6331649580b` (2026-11-20), before the base moved
to the stable branch; file paths are at that commit
(`https://github.com/WebKit/WebKit/blob/a6331649580b67f2af8a827f0eb89f73422fea55/<path>`). Line
numbers may differ on `webkitglib/2.54`, and the newest upstream pieces (for example the
Automation inspector passthrough of 2026-06-04, §2.1) must be checked on the base before a patch
relies on them. Product references: [architecture.md](../../docs/architecture.md) §4.1 "Engines"
and "Decision: our own WebKit", [privacy.md](../../docs/privacy.md), [ideas.md](../../docs/design/ideas.md), the
[README](../../README.md) roadmap (P0–P5).

Markers: **[verified]** read in the source at the pin; **[unverified]** not confirmed, needs a
spike or a test before we rely on it.

## 0. Answers first

### Passkeys / WebAuthn without Apple's browser entitlement

- **On macOS, WebKit itself never touches an authenticator for a real page.** Every request goes
  to AuthenticationServices: `WebAuthenticatorCoordinatorProxy::handleRequest` calls
  `performRequest` whenever `HAVE(UNIFIED_ASC_AUTH_UI) || HAVE(WEB_AUTHN_AS_MODERN)`, which is
  always on for macOS (`Source/WTF/wtf/PlatformHave.h:1424-1432`), then either `ASCAgent`
  (legacy) or `ASAuthorizationController` (modern, pref `WebAuthenticationASEnabled`, default
  true). Apple gates arbitrary-RP use of those APIs on
  `com.apple.developer.web-browser.public-key-credential`. So with the system `WKWebView`, and with
  an unpatched fork, a non-entitled Quickie gets no WebAuthn at all **[verified for the routing;
  the entitlement gate is Apple policy, unverified in code]**.
- **WebKit still ships its own complete authenticator stack**: `AuthenticatorManager` with
  `HidService` (USB security keys, IOKit), `NfcService`, `CcidService` (smart cards), and
  `LocalService`/`LocalAuthenticator` (Secure Enclave P-256 key per credential, Touch ID through
  `LAContext`), CTAP2/U2F drivers in `UIProcess/WebAuthentication/fido/`. On macOS it is only
  reached from the AS agent or from the `_WKWebAuthenticationPanel` SPI.
- **The fork fix is small and mostly build configuration**:
  1. build with `HAVE_UNIFIED_ASC_AUTH_UI=0` and `HAVE_WEB_AUTHN_AS_MODERN=0` (both are
     `#if !defined(...)` guarded, so an xcconfig define overrides them without touching the
     header) so `handleRequest` falls through to `authenticatorManager->handleRequest(...)`, the
     path GTK and the mock use;
  2. a ~20-line patch: `LocalAuthenticatorAccessGroup` (`WebCore/Modules/webauthn/WebAuthenticationConstants.h:92`,
     hard-coded `com.apple.webkit.webauthn`) read from our Info.plist, i.e.
     `<TEAMID>.app.quickie.webauthn`, a keychain group our Developer ID profile can grant;
  3. Quickie implements `_webView:runWebAuthenticationPanel:initiatedByFrame:completionHandler:`
     (`WKUIDelegatePrivate.h:200`) and `_WKWebAuthenticationPanelDelegate` (PIN entry, account
     picker, `requestLAContextForUserVerification`) with our own sheet.
  Result: **security keys over USB/NFC/smart card and device-bound passkeys in the Secure Enclave,
  Touch ID-protected, in our own keychain group**, the same model as Chrome's own "profile"
  passkeys on macOS. Not synced between Macs (by design; enroll a security key as the backup).
- **Not covered by WebKit's stack: hybrid (phone QR, caBLE).** There is no caBLE code in WebKit
  (`fido/` has HID, NFC, CCID drivers only) **[verified]**. Porting Chromium's `device/fido/cable`
  (BSD) is large and needs a tunnel server (Google's or Apple's) on use. Defer.
- **iCloud Keychain / Passwords-app passkeys** need the browser entitlement in every design. Apply
  for it in parallel (Apple grants it to browsers on request **[unverified for a small Developer ID
  browser]**); if granted, flip the two HAVE flags back per build, no other change.
- **No-patch path on the system WebKit (today, P0–P1):** password-manager Web Extensions through
  `WKWebExtension`. Bitwarden's MV3 extension overrides `navigator.credentials` from a page-world
  script (`apps/browser/src/autofill/fido2/content/fido2-page-script.ts`, with Safari-specific
  code paths) and stores passkeys and TOTP in the vault, so it intercepts WebAuthn wherever its
  content scripts run, which `WKWebExtension` supports **[verified in Bitwarden's source; working
  in Quickie unverified: test it]**. 1Password is closed source **[unverified]**. Also possible
  without a patch: our own page-world shim plus `_WKWebAuthenticationPanel` instances driven by the
  app for security keys only (the local authenticator still needs Apple's access group).
- **Linux (WebKitGTK):** WebAuthn is not supported. `ENABLE_WEB_AUTHN` is `OFF` by default
  (`Source/cmake/WebKitFeatures.cmake:350`), there is no Linux HID transport, and bug
  [205350](https://bugs.webkit.org/show_bug.cgi?id=205350) is open; the maintainers want the
  `credentialsd` portal (linux-credentials) rather than static device access. Options: Bitwarden
  extension (needs GTK Web Extensions, item 7), or a fork patch adding a hidraw `HidConnection`
  (or `credentialsd` client) under `ENABLE_WEB_AUTHN`; upstreamable if done as the portal client.

### WebMCP (`document.modelContext`, formerly `navigator.modelContext`)

- **Spec:** Draft Community Group Report of the W3C Web Machine Learning CG
  (`webmachinelearning/webmcp`, `index.bs`, last push 2026-11-19). Current IDL:
  `partial interface Document { [SecureContext, SameObject] readonly attribute ModelContext modelContext; }`,
  `ModelContext : EventTarget { registerTool(tool, {signal, exposedTo}); getTools(); executeTool(); ontoolchange; ontoolactivated; ontoolcancel; }`,
  tools carry `annotations` (`readOnlyHint`, `untrustedContentHint`, a consequential hint)
  **[verified in index.bs]**. `navigator.modelContext` moved to `document.modelContext`;
  `provideContext` is gone.
- **Chrome:** DevTrial from 146, origin trial 149–156 on desktop, an "Intent to Extend Experiment"
  exists, `navigator.modelContext` deprecated in 150 **[secondary sources; unverified on
  chromestatus]**. Only Gemini in Chrome consumes tools so far **[unverified]**.
- **WebKit: opposed.** `WebKit/standards-positions#670`, closed, label `position: oppose`
  (2026-06-03, concerns: duplication, privacy, security, API design, use cases, venue, ...):
  WebKit wants agent affordances in HTML/ARIA, and notes a tool's declared intent cannot be
  verified **[verified]**. No WebMCP code in WebKit **[verified: no `modelContext` IDL]**.
- **Fork implementation cost:** WebCore module `Source/WebCore/Modules/webmcp/` (IDL,
  `ModelContext`, per-`Document` tool registry, events, JSON-schema validation), a pref
  `WebMCPEnabled` (default off), IPC to the UI process (`WebPageProxy` tool-change and execute
  messages), Cocoa SPI and a GLib signal. ~2–3 k lines with tests, touching hot WebCore files
  (`Document.idl`, bindings); high rebase churn against a moving draft. Not upstreamable.
- **Containment fit:** only as untrusted input. A tool is page code; its name, description,
  schema and hints are page-authored text, i.e. a prompt-injection channel, and
  `readOnlyHint` is a claim, not a guarantee. So: expose tools only in the agent profile, label
  them untrusted, treat every call as an action that goes through approvals, and never let a
  tool's hints relax approval.
- **Recommendation: no WebCore patch.** In P4, if wanted, ship a ~200-line polyfill of
  `document.modelContext` injected into the page world of agent-profile tabs only (a `WKUserScript`
  / `webkit_user_script` at document start) that records registrations and relays
  `executeTool` through our `Automation` port. Same product value, zero engine diff, deleted
  when the spec settles.

## 1. Inventory: what the system WebKit already gives without a patch

| Need | Public API / existing SPI (macOS) | GTK/GLib API | Status |
|---|---|---|---|
| Inspector open, close, console, resources, attach/detach, element picker | `WKWebView._inspector` → `_WKInspector` `show/close/showConsole/showResources/attach/detach/toggleElementSelection/togglePageProfiling/showMainResourceForFrame:`; `_inspectorAttachmentView` (`WKWebViewPrivate.h:272,965`, `_WKInspector.h`, `_WKInspectorIBActions.h`) | `webkit_web_view_get_inspector` → `webkit_web_inspector_show/attach/detach/close` | [verified] |
| Session restore | public `WKWebView.interactionState`; SPI `_sessionStateData`, `_restoreFromSessionStateData:`, `_sessionStateWithFilter:` (`WKWebViewPrivate.h:262-267`) | `webkit_web_view_get_session_state`, `webkit_web_view_session_state_serialize`, `webkit_web_view_restore_session_state` | [verified] |
| Thumbnails | public `takeSnapshotWithConfiguration:` | `webkit_web_view_get_snapshot` | [verified] |
| Per-tab process and memory | SPI `_webProcessIdentifier`; `+[WKProcessPool _webContentProcessInfo]` → `pid`, `physicalFootprint`, CPU times, `webViews` (macOS 14.5, `WKProcessPoolPrivate.h:62-74,215`) | `get_is_web_process_responsive`; pid only via `/proc` **[unverified]** | [verified] |
| Crash reporting | public `webViewWebContentProcessDidTerminate:`; SPI `_webView:webContentProcessDidTerminateWithReason:` | signal `web-process-terminated` (`CRASHED`, `EXCEEDED_MEMORY_LIMIT`, `TERMINATED_BY_API`) | [verified] |
| Blocked-request counts | SPI `_webView:contentRuleListWithIdentifier:performedAction:forURL:` with `_WKContentRuleListAction.blockedLoad/blockedCookies/madeHTTPS/redirected/modifiedHeaders` (`WKNavigationDelegatePrivate.h:100`); DuckDuckGo uses it | `WebKitUserContentFilter`, no per-load callback **[unverified]** | [verified, macOS] |
| Request interception | none outside the inspector protocol | none | — |
| Trusted input, DOM/AX snapshots | `NSEvent` synthesis into the view (P0 stopgap); scripts in `WKContentWorld` | no event synthesis in GTK 4 | — |
| Web Extensions | `WKWebExtension*` public API (macOS 15.4+) | `WebKitWebExtension`, `...Context`, `...MatchPattern` only, GTK 4 + `ENABLE_EXPERIMENTAL_FEATURES` | see item 7 |
| Fingerprinting protections | SPI `WKWebpagePreferences._networkConnectionIntegrityPolicy` (Advanced Privacy Protections) | none | [verified] |

This corrects one assumption in `engine-architecture.md`: on macOS, WebKit's content rule lists
**do** report per-load actions, so `CAPS.blocked_counts` can be true on the system WebKit.

## 2. Patch catalogue

Legend: **Size** S < 300 lines, M 300–2 000, L > 2 000. **Rebase risk** at a WebKit rebase:
low (new files, stable seams), med (edits in active files), high (edits across hot code).

### 2.1 Agent `Automation` + `RequestGate` (P1, first)

**Key finding: upstream already has the protocol we need.** WebKit's WebDriver backend
`WebAutomationSession` (`Source/WebKit/UIProcess/Automation/`) now carries:

- `Automation.json`: browsing contexts, navigate/back/forward, `performMouseInteraction`,
  `performKeyboardInteractions`, `performInteractionSequence` (native events through
  `SimulatedInputDispatcher` and the platform `mac/`, `gtk/` input code, so events are trusted),
  `takeScreenshot`, `getComputedRole`, `getComputedLabel`, dialogs, file upload, cookies,
  permissions, virtual authenticators, `loadWebExtension`;
- **`processBidiMessage`**: WebDriver BiDi (`protocol/BidiInput.json` `performActions`,
  `BidiBrowsingContext.json` `locateNodes/getTree/captureScreenshot/navigate`, `BidiScript.json`
  `callFunction/evaluate/addPreloadScript` with sandbox realms, storage, permissions, emulation);
- **`sendInspectorMessage` / `receiveInspectorMessage`** (landed 2026-06-04, "support testing
  backend commands using Automation protocol"): passes raw Web Inspector protocol to a page's
  `WebPageInspectorController` through `InspectorPassthroughChannel`, gated by session config.
  That reaches **`Network.setInterceptionEnabled/addInterception/interceptContinue/interceptWithRequest/interceptRequestWithResponse/interceptRequestWithError`**
  and `Network.requestIntercepted {requestId, request{url, method, headers, postData}}`
  (`Source/JavaScriptCore/inspector/protocol/Network.json`) — exactly hold, inspect, continue,
  fail **[verified]**.
- Scoping: only pages with `isControlledByAutomation()` are visible to a session
  (`WebAutomationSession::getBrowsingContexts`) **[verified]**, which matches "acting happens only
  in the agent profile".

What is missing is only the **transport**: sessions are created by `RemoteInspector`
(`webinspectord` XPC on macOS, a TCP server on GTK for `WebKitWebDriver`). We forbid TCP ports and
cannot use Apple's daemon.

| | |
|---|---|
| Product need | `Automation` and `RequestGate` ports, Act mode (architecture §4.1 "Act mode is two ports"; README P1 "agent acts with real request approval"; ideas.md "Approval payload preview") |
| Change | **A1 `automation/local-channel`**: an in-process frontend for `WebAutomationSession`. New `LocalAutomationFrontendChannel : Inspector::FrontendChannel` (`ConnectionType::Local`) whose `sendMessageToFrontend` calls a client block/callback on the main run loop; SPI `-[_WKAutomationSession connectLocalClient:]` (block `void(^)(NSString *json)`), `-dispatchMessage:(NSString *)json`, `-disconnectLocalClient`; plus a config flag that enables inspector passthrough without the "testing" gate. The app creates the session itself and calls the existing `-[WKProcessPool _setAutomationSession:]` (`WKProcessPoolPrivate.h:124`); agent tabs are created with the "controlled by automation" configuration. GLib twin: `webkit_automation_session_new_local()`, `webkit_automation_session_send_message()`, signal `message-received`, `webkit_web_context_set_automation_session()`. |
| Where | `Source/WebKit/UIProcess/Automation/WebAutomationSession.{h,cpp}` (connect/disconnect already take a `FrontendChannel&`), new `LocalAutomationFrontendChannel.{h,cpp}`; `Source/WebKit/UIProcess/API/Cocoa/_WKAutomationSession{.h,.mm}`, `_WKAutomationSessionConfiguration.h`; `Source/WebKit/UIProcess/API/glib/WebKitAutomationSession.{cpp,h.in}`, `WebKitWebContext.cpp`; `Sources.txt` |
| No-patch alternative | Today's stopgap (isolated-world scripts + synthetic `NSEvent`) for `Automation`; no `RequestGate`. Hijacking `WebPageInspectorController::connectFrontend` through resolved private symbols (WebInspectorKit's `wik-bridge.mm` does this) works on the system WebKit but breaks with every OS update: spike only, never ship. |
| macOS vs GTK | Same C++ core; thin API layer per port. GTK input synthesis already exists in `UIProcess/Automation/gtk/` |
| Upstreamable | Plausible: "in-process automation client" is useful for WebKit's own API tests and embedders (Epiphany's automation mode). Propose after it works in the fork |
| Size / risk | S–M (~400 lines + tests) / low–med (WebAutomationSession is active; our seam is its existing connect API) |
| Rust side | `engine-webkit` adapter speaks JSON: Automation domain for lifecycle and input, BiDi for snapshots and scripts, inspector passthrough for `Network.*`. Generate serde types at build time from the pinned `Automation.json`, `protocol/Bidi*.json`, `Network.json` |

Playwright comparison (`webkit-research/pw-bootstrap.diff`, 347 files): `InspectorPlaywrightAgent`
+ `RemoteInspectorPipe` read NUL-delimited JSON on fds 3/4 on a `WorkQueue`;
`_WKBrowserInspector initializeRemoteInspectorPipe:headless:` starts it; its `Playwright.json`
adds contexts, pages, downloads, cookies, geolocation; `Input.json` adds trusted key/mouse/wheel/touch
(`WebPageInspectorInputAgent{Mac,Gtk}`); `Page.json` adds `createUserWorld`, `setBypassCSP`,
`crash`, file-chooser interception. Adapting it in-process is easy (replace `RemoteInspectorPipe`
with the same callback channel as A1; `connectFrontend/dispatchMessageFromFrontend` already exist),
but carrying 347 files across every rebase is the cost Playwright pays weekly with a full team.
**Decision: A1 on upstream WebAutomationSession; borrow individual Playwright pieces only when a
port test proves a gap** (candidates below).

Follow-ups, only if the conformance suite shows a gap:

| Patch | Why | Where | Size |
|---|---|---|---|
| **A2 `automation/network-scope`** | Interception lives in WebCore's `InspectorNetworkAgent` per page; loads started by service workers, `PingLoad` (beacons) and downloads may bypass a page target **[unverified]**. Playwright solved it with `ProxyingNetworkAgent` in the UI process | `Source/WebKit/UIProcess/Inspector/Agents/`, `NetworkProcess/NetworkConnectionToWebProcess.cpp` | M |
| **A3 `automation/ax-snapshot`** | A full accessibility tree in one call. First use BiDi `script.callFunction` in a sandbox realm with an aria-snapshot script (Playwright's is Apache-2.0); patch only if that is too slow | `WebCore/inspector/agents/InspectorDOMAgent.cpp` or a new `Accessibility` domain over `AXObjectCache` | M |
| **A4 `automation/agent-guards`** | Containment hardening: refuse `sendInspectorMessage` methods outside an allowlist (no `Runtime.evaluate` into the main world, no `Page.setBypassCSP`) inside the engine, not only in Rust | `WebAutomationSession.cpp` | S |

Notes: keep `navigatorWebDriverEnabled` on (honest agent; `_WKAutomationSessionConfiguration`).
The agent profile gets its own `WKWebsiteDataStore` and, if still supported, its own process
pool **[unverified: multiple pools on current WebKit]**.

### 2.2 Programmatic Web Inspector (P1)

| | |
|---|---|
| Product need | `quickie.devtools.*` palette commands per panel, dock/undock, inspect element (ideas.md "DevTools in the palette", P1); architecture `CAPS.inspector` |
| No patch | open, close, console, resources (Network/Sources tab), element picker, attach/detach, profiling, main resource for a frame: `_WKInspector` SPI on the system WebKit, used by DuckDuckGo **[verified]**. Dock target: `_inspectorAttachmentView` (dock into our own view, fits "content first"). GTK: `webkit_web_inspector_show/attach/detach/close` public |
| Patch **I1 `inspector/tabs-and-dock`** | `-[_WKInspector showTab:(NSString *)tabIdentifier]` (`elements`, `network`, `sources`, `timeline`, `storage`, `audit`, `console`, `graphics`, `layers`) and `-attachToSide:` (bottom/right/left); GLib `webkit_web_inspector_show_tab()`. Plumbing: new `ShowTab(String)` in `Source/WebKit/WebProcess/Inspector/WebInspectorUI.messages.in` next to `ShowConsole/ShowResources`, `WebInspectorUI::showTab` → `InspectorFrontendAPI.showTab` in `Source/WebInspectorUI/UserInterface/Protocol/InspectorFrontendAPI.js` (pattern of `showConsole/showResources/showTimelines`), proxy method in `UIProcess/Inspector/WebInspectorUIProxy.{h,cpp}`; dock side via existing `attachBottom/Right/Left` in the proxy |
| macOS vs GTK | Same proxy; GTK frontend window code in `UIProcess/Inspector/gtk/` |
| Upstreamable | **Yes, likely**: small, symmetric with existing IBActions. Upstream first, carry until merged |
| Size / risk | S (~150 lines) / low |

### 2.3 Content blocking with adblock-rust (P2)

| | |
|---|---|
| Product need | privacy.md §3 "Tracker and ad blocking" (network + cosmetic + scriptlets, uBO syntax), "CNAME uncloaking", origin chip blocked counter (ideas.md), `ContentBlockerPort` |
| No patch (system WebKit, now) | Compile lists to `WKContentRuleList` JSON (actions: block, block-cookies, css-display-none, notify, make-https, redirect, modify-headers, ignore-previous/following-rules; `WebCore/contentextensions/ContentExtensionActions.h`) **[verified]**; cosmetic filters and scriptlets as `WKUserScript`s in a content world from adblock-rust's per-URL resources; counts via `_WKContentRuleListAction` **[verified]**. Limits: regex subset (no lookahead, no backrefs), coarse `$domain`/`$third-party`, no `$removeparam`/`$csp`/`$redirect-rule` fidelity, slow compile of large lists, no CNAME. Good enough for P0–P1 dogfooding |
| Patch **S1 `shields/load-decider`** | A synchronous per-request hook in the **network process**, so every network load (documents, subresources, workers, beacons, WebSockets) gets one decision without an IPC hop and without moving WebKit's process model. In `NetworkLoadChecker::checkRequest` (`Source/WebKit/NetworkProcess/NetworkLoadChecker.cpp`; `NetworkResourceLoader.cpp:197-209` already wires a content-extension controller into it **[verified]**) call `ShieldsDecider::decide(url, firstPartyURL, frameURL, resourceType, pageID) → Allow / Block / Redirect(url)`. The decider is our Rust `adblock` engine behind a versioned C ABI, `dlopen`ed from `Contents/Frameworks/libquickie_shields.dylib` (path and engine bytes from the UI process through `NetworkProcessCreationParameters` and an `UpdateShieldsEngine(SharedMemory::Handle)` message), so a filter-list or adblock-rust update never needs a WebKit rebuild |
| Patch **S2 `shields/blocked-notify`** | Report blocks through the **existing** content-rule-list notification path with identifier `quickie.shields`, so the UI side keeps using `_WKContentRuleListAction` and no new API appears **[unverified: whether network-process checker results reach `WebPageProxy::contentRuleListPerformedAction`; else add one IPC message]** |
| Patch **S3 `shields/cname`** | macOS: the network process already reads `-[NSURLSessionTask _resolvedCNAMEChain]` for ITP (`NetworkDataTaskCocoa.mm:191-197`) **[verified]**, but only after connecting. Pre-request uncloaking: resolve CNAME for third-party hosts with `DNSServiceQueryRecord` (same system resolver and cache, so no new egress) before starting the task, cache per session, re-ask the decider with the canonical name. Fallback: abort at `didReceiveResponse` when the chain matches. GTK: GResolver has no CNAME record type; needs `res_query` or libsoup changes **[unverified]** |
| macOS vs GTK | S1/S2 shared C++ (NetworkProcess is cross-port); S3 per port |
| Upstreamable | S1 as a generic "embedder network decider" is unlikely upstream (Apple prefers declarative rule lists). Keep in fork; keep the seam one function |
| Size / risk | S1 M (~600 lines) / med (`NetworkLoadChecker` changes often); S2 S / low; S3 M / med |

Hardened runtime: the network process loading our dylib needs library validation to accept our
team ID, which it does when both are signed by the same team **[unverified for our XPC bundles]**.

### 2.4 Privacy hardening: trackerless defaults (P2), anti-fingerprinting hooks (P5)

Phone-home and leak inventory:

| Item | What it does | Default at pin | No-patch fix | Fork patch |
|---|---|---|---|---|
| Safe Browsing | `SafeBrowsingUtilities.mm` soft-links `SafariSafeBrowsing` (`SSBLookupContext`) | pref `SafeBrowsingEnabled` true | public `WKPreferences.fraudulentWebsiteWarningEnabled = false` | P1 default false in `UnifiedWebPreferences.yaml`; a system framework, so it may still work in our build as an opt-in **[unverified]** |
| WebPrivacy service | `Platform/cocoa/WebPrivacyHelpers.mm` asks the system `WPResources` for link-filtering, storage-access quirks, tracker rules, resource-monitor rules, high-value fraud domains; the system service fetches from Apple | used when the framework is present | none known | **P2 `privacy/no-webprivacy`**: gate `PAL::isWebPrivacyFrameworkAvailable()` behind a build flag; whether it fires for a non-Safari app at all **[unverified]**: the egress test decides |
| Private Click Measurement | sends attribution reports to ad sites; fraud-prevention tokens | `PrivateClickMeasurementEnabled` true | SPI pref off | default false |
| Speculation rules prefetch | page-requested prefetch | `SpeculationRulesPrefetchEnabled` true | pref off | default off, or same-site only |
| Link prefetch / preconnect | `<link rel=prefetch/preconnect>` | prefetch false, preconnect false (WebKit) | — | none |
| Server preconnect | preconnect on navigation start | data store `allowsServerPreconnect` | set false if the egress test flags it | — |
| ITP / resource load statistics | local classifier, no upload | on | keep on (`defaultTrackingPreventionEnabledOverride`) | — |
| Diagnostic logging | `DiagnosticLoggingClient` → `_WKDiagnosticLoggingDelegate` only if set; nothing leaves the process | — | never set the delegate | — |
| Crash reporter | `WKCrashReporter.mm` annotates local crash logs | — | system "share with developers" setting | — |
| Web Push | APNs through `webpushd` | needs Apple entitlement | leave `webPushMachServiceName` unset | — |
| Private Relay | `_allowPrivacyProxy` per page | system policy | leave | — |
| GPC | `GlobalPrivacyControlEnabled` (WebCore, optional bool, not exposed to WebKit) | Safari sends it in private browsing | none | **P2 `privacy/gpc`**: `Sec-GPC: 1` + `navigator.globalPrivacyControl` always (privacy.md §3) |
| Referrer | default `strict-origin-when-cross-origin`; ITP trims known trackers | — | — | optional: origin only cross-site |
| Link preview (force click) | fetches the link | public `allowsLinkPreview` | false (also keeps "content first") | — |
| TLS revocation | system trust evaluation may hit Apple OCSP on first visit | — | allowlist entry in the egress test | — |

Patch **P1 `privacy/defaults`**: one edit to `Source/WTF/Scripts/Preferences/UnifiedWebPreferences.yaml`
defaults (Safe Browsing, PCM, speculation prefetch), so a fresh profile is trackerless even if
the shell forgets a setting. S / low risk (YAML merges cleanly). Not upstreamable.

Anti-fingerprinting (P5):

| Hook | Upstream today | Fork patch | Size |
|---|---|---|---|
| Canvas noise | `Source/WebCore/html/CanvasNoiseInjection.{h,cpp}`, enabled through Advanced Privacy Protections (`WKWebpagePreferences._networkConnectionIntegrityPolicy`, `ScriptTrackingPrivacy*` prefs) **[verified present]** | **F1 `privacy/farbling-seed`**: seed per (site, session key) owned by Quickie, Brave-style; expose the same switch on GTK | S |
| Audio, WebGL readback | AFP covers some audio **[unverified]**; WebGL already reports a generic Apple GPU | extend F1 to `AudioBuffer`/`AnalyserNode` and `readPixels` | M |
| Fonts | macOS WebKit exposes only system fonts, not user-installed ones **[unverified at pin]** | none | — |
| Battery, MIDI, Bluetooth, USB, HID, Serial | not implemented in WebKit | none | — |

### 2.5 WebAuthn and passkeys (P3; see §0)

| Patch | Change | Where | Size / risk | Upstream |
|---|---|---|---|---|
| **W1 `webauthn/own-manager`** | xcconfig `HAVE_UNIFIED_ASC_AUTH_UI=0`, `HAVE_WEB_AUTHN_AS_MODERN=0`; `LocalAuthenticatorAccessGroup` from Info.plist key `QuickieWebAuthnAccessGroup` | `Configurations/*.xcconfig` (fork-local), `WebCore/Modules/webauthn/WebAuthenticationConstants.h`, `WebAuthenticatorCoordinatorProxy.cpp:165-183` (check the `#else` branch: it rejects cross-origin iframes) | S / low | the configurable access group might be; the routing is policy |
| Shell side | `runWebAuthenticationPanel` delegate; our sheet for PIN, account choice, Touch ID; credential management (list/delete/export) through `_WKWebAuthenticationPanel` class methods (`getAllLocalAuthenticatorCredentialsWithRPID:`, `deleteLocalAuthenticatorCredentialWithGroupAndID:credential:`, `importLocalAuthenticatorWithAccessGroup:credential:error:`) | `crates/adapters/engine-webkit`, `system` (keychain group) | M (Rust) | — |
| Signing | Developer ID provisioning profile with `keychain-access-groups` = `<TEAMID>.app.quickie.webauthn` and an `application-identifier`; Secure Enclave keys need the data-protection keychain | `engine/webkit/` sign scripts | — | **[unverified: Developer ID profiles grant this; spike first]** |
| **W2 `webauthn/linux`** (later) | `ENABLE_WEB_AUTHN=ON` for GTK plus a `credentialsd` portal client (or hidraw `HidConnection`) | `UIProcess/WebAuthentication/glib/` (new), `Source/cmake/OptionsGTK.cmake` | L / med | yes, if done as the portal client (bug 205350) |
| **W3 hybrid / caBLE** (deferred) | QR + BLE + tunnel; port from Chromium `device/fido/cable` | new `UIProcess/WebAuthentication/cable/` | L / high | unlikely |

Attestation stays `none` (the local authenticator's Apple attestation needs Apple's certificate).
Conditional mediation (autofill UI) needs our own account picker in the field's popup.

### 2.6 WebMCP (not planned; see §0)

Polyfill in the agent profile in P4 if wanted. Engine patch only if WebKit's position changes or
the CG draft stabilises and a site we use depends on it.

### 2.7 Web Extensions on WebKitGTK (P3+ for Linux)

- Status at pin: the cross-platform `WebExtensionController` core lives in
  `Source/WebKit/UIProcess/Extensions/` with GLib glue files listed in `Source/WebKit/SourcesGTK.txt`
  (`WebExtensionControllerGLib.cpp`, `WebExtensionContextGLib.cpp`, DNR SQLite store, URL scheme
  handler), but the public GLib API covers only `WebKitWebExtension`, `WebKitWebExtensionContext`
  and `WebKitWebExtensionMatchPattern` (manifest data, load background content). No controller,
  tab, window, action or permission API; `ENABLE_WK_WEB_EXTENSIONS` is on only with
  `ENABLE_EXPERIMENTAL_FEATURES` and GTK 4 (`OptionsGTK.cmake:142,198`) **[verified]**. Initial
  API landed in 2.51/2.52 **[webkitgtk.org release notes]**.
- Options, best first: (1) wait and help upstream (Igalia is exposing it; contributions are
  upstreamable and remove fork weight); (2) **E1 `extensions/glib-controller`**: our own GLib
  wrappers for controller, tab and window delegates, shaped like the Cocoa `WKWebExtensionController`
  / `WKWebExtensionTab` / `WKWebExtensionWindow` protocols, L / med risk, offered upstream;
  (3) no extensions on Linux: Bitwarden desktop + its CLI for TOTP, security keys after W2.
- Rust: one `Extensions` capability port only once a UI consumes it (engine-architecture §6).

### 2.8 Session restore, thumbnails, per-tab info, crash recovery (P1, no patch)

| Need | macOS | GTK | Patch |
|---|---|---|---|
| Back/forward state | `interactionState` (public) or `_sessionStateData`; blob is versioned by WebKit (`UIProcess/Cocoa/SessionStateCoding.mm`, `LegacySessionStateCoding.cpp`) | `webkit_web_view_get_session_state` + `_serialize` (GVariant) | none |
| Never lose a tab | keep our own URL/title/scroll per entry in `shell.sqlite` alongside the blob; restore from ours if the blob is rejected after a WebKit rebase | same | none |
| Thumbnails | `takeSnapshotWithConfiguration:`; swipe uses WebKit's `ViewSnapshotStore` | `webkit_web_view_get_snapshot` | none; hidden-tab snapshots **[unverified]** |
| Memory per tab | `_webContentProcessInfo.physicalFootprint` mapped through `webViews` | `/proc/<pid>/smaps_rollup` once pid is known; pid getter **[unverified]**, else a 10-line GLib patch | GTK maybe S |
| Crash recovery | `_webView:webContentProcessDidTerminateWithReason:` → `EngineEvent::Crashed` → "Reload" overlay, restore from session state | `web-process-terminated` | none |
| Discard background tabs | `_killWebContentProcess` / close + restore lazily | `webkit_web_view_terminate_web_process` | none |

### 2.9 Media

| Topic | macOS fork | GTK fork |
|---|---|---|
| Codecs | System AVFoundation/VideoToolbox/AudioToolbox, the same as Safari (H.264, HEVC, AAC, AV1 where the hardware decodes it); WebKit adds its own WebM parser and libwebrtc VP8/VP9 software decoders. A custom build links the same system frameworks **[verified in design; test playback]** | GStreamer: whatever the distro or Flatpak runtime ships (`gst-libav`, `codecs-extra`) |
| FairPlay | `CDMFairPlayStreaming` uses `AVContentKeySession`, a public API; WebKit's open-source Cocoa bots run FairPlay EME layout tests with a local key server ([bug 237731](https://bugs.webkit.org/show_bug.cgi?id=237731)), so FairPlay **probably still works in our build** **[unverified]**. Whether Netflix, Disney+, Apple TV serve to a non-Safari app is a service decision **[unverified]**. Test early; the architecture doc's "FairPlay is gone" may be too pessimistic | no FairPlay |
| Widevine | not in WebKit on macOS | WebKitGTK EME only through Thunder/OpenCDM for embedded; no desktop Widevine. Netflix: no |
| PiP | WebKit's video fullscreen/PiP uses system frameworks, works in third-party `WKWebView`s **[unverified for our build]** | no system PiP; skip |

### 2.10 What the system WKWebView gives that our build loses

| Loss | Impact | Mitigation |
|---|---|---|
| Automatic security updates with macOS | we own every WebKit security release | §3 cadence, patch-gap metric |
| dyld shared cache | slower cold start, more memory: our WebKit/WebCore/JSC frameworks are not prelinked | measure against the 300 ms cold-start target in P1; lazy restore |
| Apple-private entitlements of Apple's WebContent/GPU/Networking XPC services (RunningBoard process assertions, memory ownership transfer, some sandbox extensions) | background-tab priority, memory attribution and some media paths may degrade **[unverified]** | compare Activity Monitor and battery on the same pages, system vs fork, in P1 |
| FairPlay | possibly none (above) | test |
| Apple Pay | not available to a third-party browser either way | sites' Apple Pay QR flow for other browsers |
| Safe Browsing | off by our choice | Shields phishing lists (privacy.md) |
| Passkeys via iCloud Keychain, Passwords AutoFill | entitlement-gated either way | W1 local passkeys, password-manager extensions |
| Writing Tools, Look Up, data detectors | system frameworks, likely still work **[unverified]** | none |

## 3. Keeping the fork small

**Where patches live: in the fork, not here.** Every patch is a commit (or a short series) on its
own fork branch `quickie/patch/<area>-<name>` in `izevg/WebKit`, branched from `quickie/2.54`, for
example `quickie/patch/automation-local-channel`, `quickie/patch/privacy-defaults`,
`quickie/patch/webauthn-own-manager`. Areas are the groups of §2: `automation`, `inspector`,
`shields`, `privacy`, `webauthn`, `extensions`, plus `build` for build fixes and configuration.

- A patch branch is built before it merges: the `webkit` workflow takes the branch head as its
  `commit` input ([engine/README.md](../README.md), "Building a patch branch"). Reviewed and
  green, it merges into `quickie/2.54`, and this repository pins the new head of `quickie/2.54`
  in [webkit.pin](webkit.pin). No patch files, exported series or submodule are kept here.
- One commit per patch, title `[quickie][area] what`, body with the product reason, upstream bug
  link, and the WebKit tests that cover it, so `git log webkitglib/2.54..quickie/2.54` is the
  whole fork diff.
- Build configuration (`HAVE_*` overrides in fork-local xcconfig files, `ENABLE_*` CMake options)
  is a fork commit like any other.
- Prefer build flags over source edits (W1 is mostly configuration), new files over edits in hot
  files, and one seam per feature (S1 is a single call in `NetworkLoadChecker`).
- Upstream first where marked: I1, A1, W2, E1. Every merged upstream patch deletes a fork patch.

**Base and cadence.** One revision builds both ports: the WebKitGTK stable branch
(`webkitglib/2.54` now, next in ~March 2027) is the only branch that is both
([architecture.md](../../docs/architecture.md), "Decision: our own WebKit"). Igalia backports
security fixes there and publishes WSA advisories; Apple's Cocoa-only fixes are not backported
there, so on every Apple security note cherry-pick the listed bugs that touch
`Source/WebKit/*/mac|cocoa` or `WebCore/platform/*/cocoa` onto `quickie/2.54`. On a new stable
release, merge its tag into `quickie/2.54`; move to the next stable branch (`quickie/2.56`, with
every patch branch rebased) every six months. The rebase runs on every WSA advisory and Apple
security note; patch-gap target ≤ 72 h (architecture §7).

**Tests per patch** (run on the patch branch before it merges into `quickie/2.54`):

| Patch | WebKit tests | Ours |
|---|---|---|
| A1–A4 | new `Tools/TestWebKitAPI/Tests/WebKitCocoa/_WKAutomationSessionLocal.mm`, `Tests/WebKitGLib/TestAutomationSession.cpp`; `Tools/Scripts/run-webdriver-tests` (W3C WebDriver + BiDi) for regressions; `LayoutTests/http/tests/inspector/network/intercept-*` | conformance: automation, request gate, renderer crash |
| I1 | `Tests/WebKitCocoa/WKInspectorDelegate.mm` and siblings; `Tests/WebKitGLib/TestInspector.cpp` | harness: inspector opens on the right tab |
| S1–S3 | `LayoutTests/http/tests/contentextensions/`, `Tests/WebKitCocoa/ContentRuleListNotification.mm` | conformance: content blocker, blocked counts; egress test |
| privacy | `LayoutTests/privateClickMeasurement/` (expect disabled), preference defaults API tests | egress test scenarios A–C on both ports |
| W1–W2 | `LayoutTests/http/wpt/webauthn/` (mock), `Tests/WebKitCocoa/_WKWebAuthenticationPanel.mm` | manual: YubiKey, Touch ID on webauthn.io |
| all | `run-javascriptcore-tests --quick` smoke, a WPT subset | the engine conformance suite on macOS and WebKitGTK (`xvfb-run`) |

## 4. Roadmap

| Order | Item | Phase | Engine | Why now |
|---|---|---|---|---|
| 1 | Fork CI builds `quickie/2.54` for macOS (arm64; running) and GTK (x86_64, arm64), signed, with attestation | P1 | fork | everything depends on it; measure cold start, memory, FairPlay, codecs vs system WebKit |
| 2 | Session state, crash event, per-tab memory, inspector open/console via existing SPI/API | P1 | system now, fork same code | daily use; no patch |
| 3 | **A1** local automation channel (+ Rust protocol client, Automation + RequestGate on it) | P1 | fork | the P1 "done when": agent acts with real request approval |
| 4 | **I1** inspector tabs and dock (upstream PR in parallel) | P1 | fork | DevTools palette commands |
| 5 | `WKContentRuleList` Shields with `_WKContentRuleListAction` counts | P1–P2 | system and fork | blocking for daily use before S1 |
| 6 | **privacy/defaults**, **gpc**, egress test on both ports, then **no-webprivacy** if the test flags it | P2 | fork | trackerless proof |
| 7 | **S1 + S2** adblock-rust decider in the network process | P2 | fork | uBO fidelity, one decision per load |
| 8 | Password-manager Web Extension (Bitwarden) on macOS | P2–P3 | system and fork | passwords, TOTP, passkeys without any patch |
| 9 | **W1** own WebAuthn manager + Secure Enclave passkeys (spike the Developer ID keychain profile first); apply for Apple's browser entitlement in parallel | P3 | fork | the owner's OTP pain |
| 10 | **S3** CNAME uncloaking | P2–P3 | fork | parity with Brave |
| 11 | A2–A4 as conformance gaps appear | P4 | fork | external agents |
| 12 | WebMCP polyfill in the agent profile (optional) | P4 | both | no engine diff |
| 13 | **F1** farbling seeds, audio/WebGL noise | P5 | fork | privacytests.org parity |
| 14 | Linux: E1 or upstream GLib extensions API; W2 WebAuthn via `credentialsd` | Linux port | fork | Linux parity |
| — | Hybrid/caBLE, Widevine | not planned | — | cost or impossible |

## 5. Sources

- WebKit at pin: `Source/WebKit/UIProcess/Automation/{Automation.json,protocol/Bidi*.json,WebAutomationSession.h,InspectorPassthroughChannel.h}`;
  `Source/JavaScriptCore/inspector/protocol/Network.json`;
  `Source/WebKit/UIProcess/API/Cocoa/{_WKInspector.h,_WKInspectorIBActions.h,WKWebViewPrivate.h,WKNavigationDelegatePrivate.h,WKUIDelegatePrivate.h,WKProcessPoolPrivate.h,_WKAutomationSession.h,_WKAutomationSessionConfiguration.h,_WKAutomationDelegate.h,_WKWebAuthenticationPanel.h,_WKContentRuleListAction.h,_WKWebsiteDataStoreConfiguration.h,WKWebpagePreferencesPrivate.h}`;
  `Source/WebKit/UIProcess/WebAuthentication/{WebAuthenticatorCoordinatorProxy.cpp,AuthenticatorManager.cpp,Cocoa/WebAuthenticatorCoordinatorProxy.mm,Cocoa/LocalAuthenticator.mm,Cocoa/LocalConnection.mm,Cocoa/LocalService.mm,fido/}`;
  `Source/WebCore/Modules/webauthn/WebAuthenticationConstants.h`; `Source/WTF/wtf/PlatformHave.h`;
  `Source/WTF/wtf/PlatformEnableCocoa.h`; `Source/cmake/{WebKitFeatures.cmake,OptionsGTK.cmake}`;
  `Source/WebKit/SourcesGTK.txt`; `Source/WebKit/UIProcess/API/glib/{WebKitWebView.h.in,WebKitWebExtension.h.in,WebKitWebExtensionContext.h.in,WebKitAutomationSession.h.in,WebKitWebContext.h.in}`;
  `Source/WebKit/NetworkProcess/{NetworkResourceLoader.cpp,cocoa/NetworkDataTaskCocoa.mm}`;
  `Source/WebCore/contentextensions/ContentExtensionActions.h`; `Source/WebCore/html/CanvasNoiseInjection.cpp`;
  `Source/WebKit/Platform/cocoa/WebPrivacyHelpers.mm`; `Source/WebKit/UIProcess/Cocoa/SafeBrowsingUtilities.mm`;
  `Source/WTF/Scripts/Preferences/UnifiedWebPreferences.yaml`;
  `Source/WebInspectorUI/UserInterface/Protocol/InspectorFrontendAPI.js`;
  `Source/WebKit/WebProcess/Inspector/WebInspectorUI.messages.in`.
- Branches: `webkitglib/2.38` … `webkitglib/2.54`, `safari-7625.*` in `WebKit/WebKit`.
- Playwright's WebKit patch (local copy `webkit-research/pw-bootstrap.diff`): `InspectorPlaywrightAgent`,
  `RemoteInspectorPipe`, `BrowserInspectorPipe`, `_WKBrowserInspector`, `WebKitBrowserInspector`,
  `WebPageInspectorInputAgent{Mac,Gtk}`, `ProxyingNetworkAgent`, protocol `Playwright.json`, `Input.json`.
- DuckDuckGo macOS browser (local copies `webkit-research/*.swift`): `_WKInspector` use, `_WKContentRuleListAction` counts.
- WebInspectorKit (`webkit-research/wik-bridge.mm`): private-symbol inspector channel on the system WebKit.
- Bitwarden: `bitwarden/clients` `apps/browser/src/autofill/fido2/content/fido2-page-script.ts`, `fido2-content-script.ts`.
- WebMCP: `webmachinelearning/webmcp` `index.bs`; WebKit position <https://github.com/WebKit/standards-positions/issues/670>;
  Chrome: <https://developer.chrome.com/docs/ai/webmcp/imperative-api>,
  Intent to Experiment <https://groups.google.com/a/chromium.org/g/blink-dev/c/gmYffo5WOE8>,
  Intent to Extend Experiment <https://groups.google.com/a/chromium.org/g/blink-dev/c/lkdLdy5VPoA>;
  `navigator`→`document` move and Chrome 150 deprecation: secondary posts (dev.to, mcpplaygroundonline.com) **[unverified]**.
- WebKitGTK WebAuthn: <https://bugs.webkit.org/show_bug.cgi?id=205350> (open; `credentialsd` direction, 2025-10/12 comments).
- WebKitGTK extensions API: <https://webkitgtk.org/2026/03/18/webkitgtk-2.52-highlights.html>,
  webkit-dev "InjectedBundle deprecation" thread (2025-07).
- FairPlay tests on open-source builds: <https://bugs.webkit.org/show_bug.cgi?id=237731> **[unverified relevance to playback on Netflix]**.
