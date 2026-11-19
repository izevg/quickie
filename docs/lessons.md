# Lessons learned: other browsers' mistakes, our rules

Research as of 2026-11-19. **[weak]**: forums or anecdotes. **[unverified]**: no confirming source.

## Arc and Dia (The Browser Company)

| Mistake | Our rule | Source |
|---|---|---|
| "Arc was bloated. We built too much, too quickly"; for Dia they dropped TCA and SwiftUI for responsiveness | AppKit + Core Animation through `objc2`, no SwiftUI, no reactive state trees | https://browsercompany.substack.com/p/letter-to-arc-members-2025 |
| ADK, an internal SDK for native UI without touching C++, became the foundation of Dia too | A narrow `cxx` boundary + the `EnginePort` port; UI is developed on a WebKit stand-in without a Chromium build | same |
| Novel features went unused (more than one Space: 5.5 % of DAU, Live Folders: 4 %) | Built for one user, no concepts for their own sake; every feature is a palette command, not a panel | same |
| Swift on Windows cost them a fork of the toolchain | macOS only | https://speakinginswift.substack.com/p/swift-meet-winrt |
| CVE-2024-45489: Firebase rules let an attacker push a Boost with JavaScript to another user | No cloud content that executes code; no sync | https://arc.net/blog/CVE-2024-45489-incident-response |
| Syncing a "archive after 24 h" setting wiped ~30 tabs on another Mac; "Never lose a tab" broken | Closing goes to a 30-day trash; no destructive defaults; session migrations are tested | https://browsercompany.substack.com/p/there-and-back-again-the-product |
| Arc shuts down on 2026-12-22, citing the cost of keeping a browser secure | Minimal diff, automated rebases, a patch-gap metric | https://techcrunch.com/2026/10/22/arc-browser-sunsets-december-22-as-the-browser-company-doubles-down-on-dia/ |
| Dia CVE-2025-13132 (fullscreen without a notice), CVE-2025-15032 (a window without about:blank in the URL): typical custom-shell bugs | Fullscreen always shows a notice; origin chip on every origin change; spoofing tests in CI | https://diabrowser.com/security/bulletins |
| Dia: prompt-injection detection is unreliable; allowlists fall to open redirects | Provenance rule: the agent only visits URLs from the user's context | same |
| Arc performance: RAM, battery, lag **[weak]** | Frame budgets in CI performance tests; background tabs discarded | reddit r/ArcBrowser |
| Arc's lag came from SwiftUI and a busy main thread, not from composition | Native composition: no compositor of our own; Metal only inside one widget, after a measurement shows Core Animation missing the budget | same |

## Vivaldi

| Mistake | Our rule | Source |
|---|---|---|
| React UI: the chrome is slower than the pages **[weak]** | No HTML chrome | https://forum.vivaldi.net/topic/60087 |
| The MRU Ctrl+Tab order breaks on fast presses, the cycler flashes **[weak]** | MRU updates synchronously on activation; the overlay appears only after a hold > 150 ms; a quick tap switches to the previous tab without any UI | https://forum.vivaldi.net/post/848287 |
| A daily ping with a unique ID | Zero telemetry | https://vivaldi.com/privacy/browser/ |

## Orion (WebKit)

| Mistake | Our rule | Source |
|---|---|---|
| ~70 % of extensions work, 1Password and Grammarly broke; one missing API is enough | Chromium: MV3 extensions natively | https://help.kagi.com/orion/browser-extensions/macos-extensions.html |

## Zen, Helium, Thorium (small forks)

| Mistake | Our rule | Source |
|---|---|---|
| Zen: workspaces and tabs lost after updates | Session migrations are tested against real snapshots of earlier versions before a release | https://www.reddit.com/r/zen_browser/comments/1iwuwq2/ |
| Helium ships a release "within hours" thanks to rented CI runners | Own runner + optional remote execution; rebase bot | https://github.com/imputnet/helium |
| Thorium: a solo maintainer fell almost a year behind | Extended Stable (8 weeks), minimal patches, agent-assisted rebases | https://www.codegenes.net/blog/thorium-browser-download/ |
| The patch gap of forks is measurable and public | Patch-gap metric ≤ 72 h | https://chromium-drift.pages.dev/ |

## Chromium on macOS

| Mistake | Our rule | Source |
|---|---|---|
| Electron overrode the private `_cornerMask`, loading WindowServer on Tahoe | Never override private AppKit methods | https://releases.electronjs.org/pr/48376 |
| 120 Hz through CADisplayLink sat behind a feature flag (M139) | Check and enable `kCADisplayLink` in our build **[verify the current default]** | `ui/display/mac/display_link_mac.mm` |
| Atlas (Chromium): Cmd+C/V broke on a Russian layout | Bindings by key code + ASCII-capable layout; tests with a Russian layout | https://community.openai.com/t/hotkey-issue-in-atlas-browser-when-using-russian-keyboard-layout-copy-paste-stops-working/1367261 |
| `trackSwipeEvent` can block events for an unpredictable time (comment in `HistorySwiper`) | Swipe through the engine + a native preview; check `isSwipeTrackingFromScrollEventsEnabled` | `history_swiper.h` |
| MV2 removed (off in M139, code deleted in M150–151, Web Store purged on 2026-08-31) | No MV2; blocking is native adblock-rust | https://github.com/w3c/webextensions/issues/1000 |
| `content_shell` / the content API without `chrome/` has no extensions or profiles | Fork with `chrome/`, replace only the window | https://chromium.googlesource.com/chromium/src/+/master/content/ |

## Liquid Glass (macOS 26)

| Mistake | Our rule | Source |
|---|---|---|
| HIG: glass only in the navigation/controls layer, never in content; no glass on glass | The E0–E3 model ([architecture.md](architecture.md), §4.2) | https://developer.apple.com/design/human-interface-guidelines/materials |
| `NSGlassEffectView` does not update when windows move under it (26.2); resize lag | Glass only on floating elements, never on window edges; a resize test | https://developer.apple.com/forums/thread/810314 |
| Legibility complaints; 26.1 added a Clear/Tinted switch | Test both modes; E3 dims content by 25–35 % | https://www.macrumors.com/2025/11/03/apple-releases-macos-tahoe-26-1/ |

## Agentic browsers (Comet, Atlas, Opera Neon, Dia)

| Mistake | Our rule | Source |
|---|---|---|
| CometJacking: a prompt in a URL parameter leaked memory and connector data | The agent never sees data of the main profile when acting | https://www.akamai.com/blog/security/cometjacking-how-one-click-can-turn-perplexitys-comet-ai-browser-against-you |
| Invisible text in screenshots injected instructions | Page content is always marked untrusted and never mixed with user instructions | https://brave.com/blog/unseeable-prompt-injections/ |
| Atlas: the omnibox treated an invalid URL as a trusted prompt | Palette: `?` is the explicit agent prefix; otherwise input never goes to an LLM | https://www.theregister.com/2025/10/27/openai_atlas_prompt_injection/ |
| Chrome 136+: `--remote-debugging-port` is ignored on the default profile because of cookie theft | Only an internal CDP channel, only the agent profile, no TCP | https://developer.chrome.com/blog/remote-debugging-port |
| Playwright MCP: origin allow/block lists are not a security boundary | The boundary is the profile + approvals + provenance | https://playwright.dev/mcp/configuration/options |
| Brave: isolated profile, start on request, no internal or non-HTTPS pages, a second model check | Adopt all of it; the second model is optional | https://brave.com/blog/ai-browsing/ |
