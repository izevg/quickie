# Google Stitch prompt

How to use: paste the "Global brief" once, then generate one screen at a time by adding that
screen's block. Every element carries a `[FSD: layer/slice]` tag; those tags are how the mockups
get cut into components later ([architecture.md](../architecture.md), §4.2).

## Global brief (paste first)

```
Design reference mockups for "Quickie", a personal desktop web browser for macOS 26 (Tahoe), 1512x982 window (MacBook Pro 14), light AND dark variants of every screen.

CORE CONCEPT
- Content is everything. In the idle state the website fills the ENTIRE window edge to edge: no tab bar, no toolbar, no sidebar, no address bar, no visible window title bar or traffic-light buttons, no bookmarks bar.
- UI appears ONLY on explicit keyboard/gesture invocation and floats above the page. Never design hover-revealed panels or edge-reveal bars.
- Keyboard-first like VS Code: command palette, fuzzy search, keycaps shown next to every command.

VISUAL LANGUAGE — Apple Liquid Glass with strict elevation
- E0: web content (use realistic sample sites: a news article, a GitHub-like code page, a docs page). No glass.
- E1: tiny transient indicators (2px load progress line at top edge, small toast pills, origin chip). Clear thin glass, no shadow.
- E2: floating overlays (palette, switcher, find bar, popovers). Regular Liquid Glass, 20-24px corner radius, soft medium shadow, page behind dimmed ~10%.
- E3: modal security/permission sheets. Denser tinted glass, deep shadow, page behind dimmed ~30%.
- Rules: never glass on glass; only one E2 overlay visible at a time; no decorative glass; glass only where something floats above content.
- Typography: SF Pro Text/Display, SF Mono for URLs, shortcuts and code. Keycaps rendered as small rounded glyph chips (⌘ ⇧ ⌥ ⌃ ⏎ ⎋).
- Color: system neutrals, single accent (system blue), semantic red for destructive/security, amber for agent activity.
- Motion hint (annotate, don't animate): spring scale 0.96→1 + fade for E2, slide-up for E3.

ANNOTATION
- Label every component with a small tag in the margin using this format: [FSD: layer/slice], e.g. [FSD: widgets/command-palette], [FSD: entities/tab-row], [FSD: shared/keycap].
- Layers: app, screens, widgets, features, entities, shared.
```

## Screens (add one at a time after the global brief)

### S1. Idle

```
Screen S1 "Idle": a long-form news article fills the whole window. Nothing else except a 2px accent progress line at the very top at 70% [FSD: widgets/load-progress-line]. Also a variant with a tiny E1 origin chip "github.com 🔒" fading at top-center after cross-origin navigation [FSD: widgets/origin-chip].
```

### S2. Address overlay (⌘L)

```
Screen S2 "Address overlay": centered E2 glass field 640px wide, ~20% from top. Shows full current URL in SF Mono with the host emphasized, lock icon, site shields counter "12 blocked" [FSD: entities/origin-label], [FSD: features/toggle-shields]. Under it: 4 action rows with keycaps: Copy URL ⌘⇧C, Copy as Markdown ⌘⌥C, Open in Safari, Site settings [FSD: entities/command-row]. No search suggestions.
```

### S3. Command palette (⌘⇧P)

```
Screen S3 "Command palette": E2 glass panel 720px wide near top. Input shows "> rel". Results grouped: "Commands" (Reload ⌘R, Reopen closed tab ⌘⇧T, Reload without cache ⌘⇧R), "Recently used". Each row: icon, title with fuzzy-matched letters highlighted, category, right-aligned keycaps [FSD: entities/command-row], [FSD: shared/keycap]. Footer hint line: "> commands · @ headings · # find · ? ask agent · ! search engines" [FSD: widgets/command-palette].
```

### S4. Go to tab or history (⌘P)

```
Screen S4 "Go to": same E2 palette without prefix, query "pr review". Sections "Open tabs" (favicon, title, host in SF Mono, "2m ago", pinned slot badge "⌘3") and "History" (dimmer rows). Selected row has subtle accent fill, not glass [FSD: entities/tab-row], [FSD: entities/history-row], [FSD: widgets/command-palette].
```

### S5. MRU tab switcher (hold ⌃⇥)

```
Screen S5 "Tab switcher": E2 glass strip centered, horizontal row of 6 tab thumbnails (16:10, rounded 12px) in most-recently-used order, second one selected with accent ring, title + host under each. Above strip a small inline filter field showing typed "doc". Hint: "release ⌃ to switch · W close · ⇧ reverse" [FSD: widgets/tab-switcher], [FSD: entities/tab-thumbnail].
```

### S6. Tab overview (pinch)

```
Screen S6 "Tab overview": page zoomed out; grid of all open tabs as live thumbnails on a dimmed blurred backdrop (single E2 container, not one glass per card), pinned tabs first with ⌘1–⌘9 badges, close buttons appear only on the focused card (keyboard focus, not hover) [FSD: widgets/tab-overview].
```

### S7. Find in page (⌘F) and link hints (F)

```
Screen S7a "Find": compact E2 pill top-right: query, "3 / 17", up/down, case and regex toggles [FSD: widgets/find-bar]. Page has highlighted matches.
Screen S7b "Link hints": every visible link on a docs page gets a tiny yellow-ish label with 1-2 letters (A, S, DF…) near it; small E1 hint at bottom "type letters · ⇧ open in background · ⎋ cancel" [FSD: widgets/link-hints-layer].
```

### S8. Permission and security sheet

```
Screen S8 "Permission": E3 sheet attached to top-center: big origin "meet.example.com" in SF Mono, request "Use camera and microphone", buttons "Allow once", "Allow on this site", "Block" (Block = default, ⎋). Variant b: "example.com is now full screen — press ⎋ to exit" notice [FSD: widgets/permission-sheet].
```

### S9. Shields and downloads popovers

```
Screen S9a "Shields": E2 popover from top-right: site host, toggle Shields, counters (trackers 23, ads 41, fingerprinting attempts 3), per-site toggles, link "Filter lists…" [FSD: widgets/shields-popover].
Screen S9b "Downloads": E2 popover list with progress bars, reveal-in-Finder action, keycaps [FSD: widgets/downloads-popover], [FSD: entities/download-row].
```

### S10. AI agent

```
Screen S10a "Agent tab": a tab controlled by an AI agent: thin 2px amber inner border around the whole content, small E1 amber pill top-center "Agent · Claude Code · reading page" with Stop ⌘. [FSD: entities/agent-badge].
Screen S10b "Agent approval": E3 sheet: "Claude Code wants to submit a form on shop.example.com", shows the exact action (field values in SF Mono), source of the URL ("from your open tab"), buttons "Approve", "Deny", checkbox "Approve similar actions for 10 minutes" [FSD: widgets/agent-approval-sheet].
Screen S10c "Ask agent": the E2 command palette expanded into an answer panel. Input row: "? summarize this PR and list risky changes". Below: streaming markdown answer with a blinking caret, inline code in SF Mono, a "Sources" footer with chips pointing to sections of the current page. Top-right small chip with the model and endpoint "ollama · qwen3:32b · local" [FSD: entities/model-chip]. Actions row with keycaps: Copy ⌘C, Follow-up ⏎, "Act on this page…" (opens agent-profile tab) [FSD: widgets/agent-answer-panel], [FSD: features/ask-agent]. Also an empty-state variant: "No LLM endpoint configured — Settings › Agents".
```

### S11. Settings and keymap editor

```
Screen S11a "Settings": standard macOS settings window (sidebar + content, native controls, no excessive glass): General, Privacy & Shields, Keyboard, Agents, Advanced [FSD: screens/settings].
Screen S11b "Keymap editor" (VS Code-like): searchable table with columns Command, Keybinding (keycaps), When (monospace expression like "pageFocused && !inputFocused"), Source (Default/User). One row in "recording" state: "Press desired key combination…". Conflict warning row in red [FSD: screens/keymap-editor], [FSD: features/record-shortcut].
```

### S12. Toasts

```
Screen S12 "Toasts": stack of E1 pills bottom-center: "URL copied", "Tab restored · ⌘⇧T", "Shields blocked 12 requests" [FSD: widgets/toast-stack].
```

## After generation

Done for the first round (2026-11-19):

1. The exported HTML lives in [mocks/](mocks/), grouped as `mocks/<group>/<screen>/{light,dark}.html`
   (groups: content, navigation, system, agent, settings; sticker sheets in `mocks/components/`).
2. The component inventory, cut into FSD layers: [inventory.md](inventory.md). The tokens, mapped
   to native macOS: [system.md](system.md). The generator's ideas, with verdicts: [ideas.md](ideas.md).
3. Stitch's HTML is only a reference for proportions, tokens, and copy; the implementation is
   AppKit through `objc2` in the `crates/ui/*` crates (`GlassSurface::new(Elevation::E2)` and so on).
