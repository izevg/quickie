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
| **Throwaway jars** instead of container tabs | user decision | `Open in Throwaway Jar` and `Reopen in Throwaway Jar`: the tab gets its own ephemeral cookie and storage partition that dies with the tab. No groups, no profiles, nothing to manage; the origin chip marks the tab. `⌘⇧N` opens a new tab in a fresh jar, so it also replaces the private window | P3 |
| **DevTools in the palette** | user decision | A `quickie.devtools.*` command per panel (Elements, Console, Network, Sources, Performance, Application, Memory), plus inspect element, dock or undock, device mode, disable cache, clear site data. Chrome's `⌥⌘I`, `⌥⌘J`, `⌥⌘C` stay; the rest are palette commands, rebindable | P1 |
| **Agent skills** | user decision | A skill is a named recipe: prompt, allowed tools, output form (`Synthesize Page Citations`, `Summarize`, `Extract Table`, `Compare Tabs`, `Explain Code`). A built-in set ships; user skills are files. Each one is a palette command under `?`. The shape follows A2A's `AgentSkill` (id, name, description, tags, examples) so it can be published later | P4 |
| **Bookmarks without a bar** | user decision | `⌘D` saves the page and opens a toast where `⇥` adds tags; a second `⌘D` edits or removes. Saved pages rank first in `⌘P` with a mark; `#tag` filters by tag; "read later" is a tag. Pinned slots `⌘1` … `⌘9` are the favorites. The library sheet searches, retags, imports and exports HTML. No bar, no folder tree | P3 |
| **Search shortcuts** | user decision | `!` in the palette runs a keyword search (`!gh quickie`); bookmarklets become commands | P3 |
| **Sync between Macs** | user decision | None at first: a one-time import from Chrome (bookmarks, history, passwords as CSV). Passwords and passkeys stay with the macOS Passwords app or a password manager. Later, only if needed: an end-to-end encrypted log of the library in a folder you choose, key in the Keychain, no server. Chrome Sync is out: it needs Google keys and hands Google the data | P5 |

## 0.3.0 ideas

- **Inspector accessibility audits.** Add an axe-core suite to the existing Web Inspector
  Audit tab. Start with the open document and link findings to their DOM nodes. Run checks
  on request and keep reports local. Surface findings that need manual review.
  [WebKit Audit](https://webkit.org/web-inspector/audit-tab/) ·
  [axe-core](https://github.com/dequelabs/axe-core)

## Later

| Idea | Note |
|---|---|
| **MCP connectors for the built-in agent** | The built-in agent as an MCP client of servers you configure, so a skill can reach beyond the page. The reverse of `quickie-mcp`, which lets external agents drive the browser. Same containment: acting only in the agent profile, approvals for irreversible calls, the server list is explicit |

## Reject

| Claim | Why |
|---|---|
| `⌘T` opens the tab switcher (idle page copy) | `⌘T` is a new tab everywhere on macOS; the switcher stays on `⌃⇥` |
| Site settings on `⌘,` (address overlay) | `⌘,` opens the app's settings on macOS; site settings get their own command |
| Geist and JetBrains Mono | A native app uses SF Pro and SF Mono ([system.md](system.md#typography)) |
| "100 % Safe", "undetectable jitter", "stops calls before the TCP handshake" | Claims we cannot prove; the UI shows measured facts only |
| An agent that indexes pages in the background ("Agent indexed 18 symbols") | Agents act only on an explicit request; nothing runs on its own |
| A Metal compositor, `LiquidGlassWindow`, `.windowChromePresentation`, "Metal 4.1" | Invented APIs. And a compositor of our own would be slower: Chromium already renders with Metal and hands frames to WindowServer as layers, so a second full-window pass adds latency and power, loses hardware video overlays, and loses the system glass, text, input methods, and accessibility. See the native composition invariant in [AGENTS.md](../../AGENTS.md#product-invariants) |
| "Claude API" as a direct backend | The built-in agent speaks the OpenAI-compatible API; anything else goes through such an endpoint or through MCP |
| Container-tab settings, version label `v1.4.2`, the "N=420 study" | Placeholder content |
