# Ideas from the Stitch mockups

The generator wrote copy and feature claims into the mockups. Some are worth building, some
conflict with our decisions, and some are invented. Each gets a verdict. "Adopt" items land in the
phase given; the rest stay here as a record.

## Adopt

| Idea | Where it appeared | What we build | Phase |
|---|---|---|---|
| **Environment chips** (`PROD`, `STAGE`, `LOCALHOST:3000`) | voice guide | Host patterns in settings map to an environment; the origin chip, address overlay, and tab rows show it; `PROD` can tint the agent frame red and require approval for every agent action | P3 |
| **Quick Dismiss**: auto-reject cookie banners | settings › privacy | Cookie-notice filter lists plus click-to-reject rules for common consent platforms, inside Shields | P2 |
| **HTTPS-only, per-site JavaScript off** | shields popover | Per-site toggles in `SitePolicy` | P2 |
| **Reader mode** (`Toggle Reader Mode`) | voice guide, toasts page | A palette command that reflows the page; fits "content is sovereign" | P3 |
| **`⌘K` as an alias for the palette** | idle page copy | Default binding next to `⌘⇧P`; muscle memory from Linear, Arc, Raycast | P0 |
| **Numbered answers in sheets** | permission sheet | `1` allow once, `2` allow on this site, `⎋` the safe default | P3 |
| **Origin chip on tab switch too**, with the blocked counter | origin chip sheet | Not only after cross-origin navigation; holds 2.4 s | P0 |
| **Agent transparency**: plan and progress | agent tab | The agent badge shows the plan line and progress (`reading page · 84 %`) | P4 |
| **Citations anchored to the page** | ask agent | `Sources from this page` chips scroll to and highlight the cited span | P4 |
| **Speed readout** | ask agent | `24 tok/s · 0.4 s TTFT` under the answer, measured | P4 |
| **Detect local LLM** | ask, empty state | A button that probes Ollama (`127.0.0.1:11434`) and LM Studio on loopback, only when clicked | P4 |
| **Approval payload preview** | approval sheet | Method, endpoint, fields with secrets masked (`●●●● 4242`), URL provenance, scoped "approve similar for 10 min on this domain" | P4 |
| **`Stop ⌘.`** for agents | agent tab | macOS's cancel convention | P4 |
| **Command ids** `quickie.<area>.<action>` | keymap editor | Stable ids in `domain/commands` (`quickie.tab.reopenClosed`), shown in the keymap editor, used in `keybindings.json` | P0 |
| **Keymap editor details** | keymap editor | Recording state, conflict row in red, `Open keybindings.json`, user vs default source | P3 |
| **Downloads on `⌘J`**, reveal in Finder | downloads popover | Standard binding | P3 |
| **Voice** | everywhere | Discreet and cheeky: "Protection is on", "Wrapped and protected by default", "No data leaves your Mac" ([system.md](system.md#voice)) | P0 |

## Ask first

| Idea | Why it needs a decision |
|---|---|
| **Container tabs** (isolated cookie jars per tab group) | Real privacy value, but it is a grouping concept, and Spaces and profiles were rejected as clutter. A lighter form: "open in a throwaway jar" as a command, no visible groups. |
| **Inspect Network Requests** as a palette command | Opens DevTools on the Network panel. Cheap with Chromium; only useful if DevTools stays docked-off as a window. |
| **Synthesize Page Citations** as an agent command | A canned Ask prompt; fine as a command, but every canned prompt is a feature to maintain. |

## Reject

| Claim | Why |
|---|---|
| `⌘T` opens the tab switcher (idle page copy) | `⌘T` is a new tab everywhere on macOS; the switcher stays on `⌃⇥` |
| Site settings on `⌘,` (address overlay) | `⌘,` opens the app's settings on macOS; site settings get their own command |
| Geist and JetBrains Mono | A native app uses SF Pro and SF Mono ([system.md](system.md#typography)) |
| "100 % Safe", "undetectable jitter", "stops calls before the TCP handshake" | Claims we cannot prove; the UI shows measured facts only |
| An agent that indexes pages in the background ("Agent indexed 18 symbols") | Agents act only on an explicit request; nothing runs on its own |
| A Metal compositor, `LiquidGlassWindow`, `.windowChromePresentation`, "Metal 4.1" | Invented APIs in the sample pages; the shell uses AppKit and Core Animation |
| "Claude API" as a direct backend | The built-in agent speaks the OpenAI-compatible API; anything else goes through such an endpoint or through MCP |
| Container-tab settings, version label `v1.4.2`, the "N=420 study" | Placeholder content |
