# Component inventory (FSD)

Every component in the Stitch mockups ([mocks/](mocks/)), cut into Feature-Sliced Design layers.
The `[FSD: …]` tags in the mockups were a first guess; this table is the decision. Names are the
Rust module names in the `crates/ui/*` crates. Tokens: [system.md](system.md).

## Mockups

| Group | Screen | Mockup (light / dark) | Widgets on it |
|---|---|---|---|
| content | Idle, full bleed | [idle](mocks/content/idle/light.html) | `load_progress_line` |
| content | Origin chip after navigation | [origin-chip](mocks/content/origin-chip/light.html) | `origin_chip` |
| content | E1 indicators | [indicators](mocks/content/indicators/light.html) | `origin_chip`, `toast_stack`, `load_progress_line` |
| content | Toast stack | [toasts](mocks/content/toasts/light.html) | `toast_stack` |
| navigation | Address overlay `⌘L` | [address](mocks/navigation/address/light.html) | `address_overlay` |
| navigation | Command palette `⌘⇧P` | [commands](mocks/navigation/commands/light.html) | `command_palette` (commands mode) |
| navigation | Go to `⌘P` | [go-to](mocks/navigation/go-to/light.html) | `command_palette` (go-to mode) |
| navigation | MRU switcher `⌃⇥` | [tab-switcher](mocks/navigation/tab-switcher/light.html) | `tab_switcher` |
| navigation | Tab overview (pinch) | [tab-overview](mocks/navigation/tab-overview/light.html) | `tab_overview` |
| navigation | Find `⌘F` | [find](mocks/navigation/find/light.html) | `find_bar` |
| navigation | Link hints `F` | [link-hints](mocks/navigation/link-hints/light.html) | `link_hints_layer` |
| system | Permission sheet | [permission](mocks/system/permission/light.html) | `permission_sheet` |
| system | Full-screen notice | [fullscreen](mocks/system/fullscreen/light.html) | `fullscreen_notice` |
| system | Shields popover | [shields](mocks/system/shields/light.html) | `shields_popover` |
| system | Downloads popover `⌘J` | [downloads](mocks/system/downloads/light.html) | `downloads_popover` |
| agent | Agent-controlled tab | [session](mocks/agent/session/light.html) | `agent_frame` |
| agent | Approval sheet | [approval](mocks/agent/approval/light.html) | `agent_approval_sheet` |
| agent | Ask agent `?` | [ask](mocks/agent/ask/light.html) | `command_palette` (ask mode) → `agent_answer_panel` |
| agent | Ask agent, no endpoint | [ask-empty](mocks/agent/ask-empty/light.html) | `agent_answer_panel` (empty state) |
| settings | Settings window | [general](mocks/settings/general/light.html) | screen `settings` |
| settings | Keymap editor | [keymap](mocks/settings/keymap/light.html) | screen `settings`, pane `keyboard` |

Each folder also has `dark.html`. Sticker sheets of single components: [mocks/components](mocks/components/).

## shared (no domain knowledge)

`keycap`, `pill`, and `hint_bar` live in `controls/`, with the styled text they are set in.

| Module | What | States / variants | Seen in |
|---|---|---|---|
| `glass` | `GlassSurface::new(Elevation)`: material, shadow, ring, dimming scrim | E1, E2, E3; macOS 15 fallback | everywhere |
| `keycap` | One glyph chip; `KeycapGroup` for a chord | default, on a selected row (accent border) | [keycap](mocks/components/keycap.html) |
| `pill` | Rounded capsule with icon + text | neutral, success, destructive, agent | toasts, origin chip, find bar, status chips |
| `list` | `NSTableView` wrapper with a diffing source, keyboard selection, section headers | — | palette, popovers, keymap |
| `hint_bar` | Footer of key hints (`↑↓ select · ⏎ open · ⎋ close`) | — | every E2 |
| `progress` | Thin line and ring progress | determinate, indeterminate | load line, downloads, agent ingestion |
| `toggle_row` | Label + detail + switch | on, off, disabled | shields, settings |
| `motion` | Timing tokens and helpers (`instant`, `overlay_enter`, `sheet_enter`, `fade`, `gesture_settle`); picks `instant` for keyboard-initiated presentation | Reduce Motion | everywhere |

## entities (one domain object, shown)

| Module | Domain object | States | Seen in |
|---|---|---|---|
| `command_row` | `Command` + its binding | default, selected (accent tint), fuzzy-highlighted title, category tag | [command-row](mocks/components/command-row.html), palette |
| `tab_row` | `Tab` | default, selected, pinned slot badge `⌘3`, age | go-to |
| `tab_thumbnail` | `Tab` | default, selected ring, focused (`W` to close), pinned badge | switcher, overview |
| `history_row` | `HistoryEntry` | default, selected, visit count | go-to |
| `origin_label` | origin of a URL | secure, insecure, environment chip, blocked counter | origin chip, address |
| `environment_chip` | host pattern → `PROD` / `STAGE` / `LOCAL` | colors by environment | origin chip, address, tab rows |
| `download_row` | `Download` | in progress (speed, ETA), complete, failed | downloads |
| `filter_list_row` | `FilterListSet` entry | enabled, updating, rule count, updated ago | settings › privacy |
| `binding_row` | `KeyBinding` | default, recording, conflict, user vs default source | keymap editor |
| `agent_badge` | `AgentSession` | reading, acting, waiting for approval, progress | agent session |
| `model_chip` | `LlmEndpoint` | local, remote, unreachable | ask agent |
| `citation_chip` | a source span on the page | default, hovered by keyboard | ask agent |

## features (one user action)

| Module | Action | Trigger |
|---|---|---|
| `copy_url`, `copy_markdown_link` | Copy the URL / a Markdown link | `⌘⇧C`, `⌘⌥C` |
| `open_in_safari` | Hand the page to Safari | address overlay |
| `reopen_closed_tab` | Restore from the trash | `⌘⇧T` |
| `switch_tab`, `close_tab` | MRU switch, close (to the trash) | `⌃⇥`, `W` in switcher and overview |
| `find_in_page` | Query, next, previous, case, regex | `⌘F`, `⏎`, `⇧⏎` |
| `follow_link_hint` | Type hint letters, `⇧` for background | `F` |
| `toggle_shields`, `set_site_protection` | Shields on/off, per-site toggles | shields popover |
| `answer_permission` | Allow once `1`, allow on site `2`, block `⎋` (default) | permission sheet |
| `record_shortcut` | Record a chord, detect conflicts | keymap editor |
| `ask_agent`, `follow_up`, `act_on_page` | Ask, continue, switch to Act | `?`, `⏎`, `⌥⏎` |
| `approve_agent_action` | Approve, deny, approve similar for 10 min on this domain | approval sheet `⏎` / `⎋` |
| `stop_agent` | Stop the session | `⌘.` |
| `detect_local_llm` | Probe Ollama and LM Studio on loopback, only when clicked | empty state |
| `reveal_download` | Show in Finder | downloads popover |

## widgets (a composed block)

Grouped by meaning so that no folder breaks the rule of four. A group is a folder of slices, and
the slices inside a group still never use each other.

| Group | Slices |
|---|---|
| `palette` | `command_palette` (modes: commands, go-to, ask), `address_overlay` |
| `tabs` | `tab_switcher`, `tab_overview` |
| `page` | `find_bar`, `load_progress_line`, `origin_chip` |
| `hints` | `link_hints_layer` (labels drawn natively above the page, never in its DOM) |
| `feedback` | `toast_stack`, `fullscreen_notice` |
| `sheets` | `permission_sheet`, `agent_approval_sheet` |
| `popovers` | `shields_popover`, `downloads_popover` |
| `agent` | `agent_frame` (2 px border + badge), `agent_answer_panel` (streaming, citations, empty state) |

## screens and app

| Layer | Module | What |
|---|---|---|
| screens | `browser` | The window: E0 page plus the overlay host |
| screens | `settings` | Standard settings window; panes `general`, `privacy`, `keyboard` (keymap editor), `agents`, `advanced` |
| app | `overlay_router` | One E2/E3 at a time, focus return, `⎋` handling |
| app | `theme` | Light/dark/tinted, Increase Contrast, Reduce Motion |

## Follow-ups for the code

- The mockups' window chrome, sample pages, and API names (`LiquidGlassWindow`,
  `.windowChromePresentation`, a Metal compositor) are invented by the generator: reference for
  look and copy only, never for APIs.
