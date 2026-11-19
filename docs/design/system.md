# Design system

The tokens Quickie's UI is built from, taken from the Stitch design system
([mocks/stitch-design-system.md](mocks/stitch-design-system.md)) and mapped onto native macOS. The
mockups are web pages, so they approximate what AppKit gives for free; **the native column wins**.
Tokens live in `crates/ui/shared` as Rust constants; nothing hard-codes a value elsewhere.

## Principle

**Content is sovereign.** In the idle state the page owns every pixel. UI appears on a deliberate
key, gesture, or click, floats on glass above the page, and evaporates when the action ends.

## Color

Semantic colors only. They follow the user's accent color, light and dark mode, Increase
Contrast, and the Clear/Tinted glass setting without any work on our side.

| Token | Stitch (light / dark) | Native | Use |
|---|---|---|---|
| `accent` | `#0071E3` / `#0A84FF` | `NSColor.controlAccentColor` | Selection fill, focus ring, primary action |
| `destructive` | `#FF3B30` | `NSColor.systemRed` | Block, deny, revoke, conflicts, `PROD` environment |
| `agent` | `#FF9F0A` | `NSColor.systemOrange` | Everything an agent does: border, badge, progress |
| `success` | `#34C759` | `NSColor.systemGreen` | Protection on, download done, `LOCAL` environment |
| `label` / `secondary-label` | `#1D1D1F` / `#6E6E73` | `labelColor` / `secondaryLabelColor` | Text on glass |
| `separator` | `#D2D2D7` / `#48484A` | `separatorColor` | Hairlines inside overlays |
| `surface*` | `#F5F5F7` … `#3A3A3C` | none: glass materials (see Elevation) | — |
| `selection-fill` | accent at 100 % behind the selected row | `controlAccentColor`, text `alternateSelectedControlTextColor` | Selected palette row: fill, never glass |

The `content-*` and `syntax-*` colors in the Stitch file describe the sample web pages, not the
browser; they are not tokens.

## Typography

Stitch used Geist and JetBrains Mono because it renders web pages. Quickie is native: **SF Pro and
SF Mono** through `NSFont.preferredFont(forTextStyle:)` and `monospacedSystemFont`, which keeps
Dynamic Type and optical sizes. No bundled fonts.

| Role | Stitch | Native text style | Where |
|---|---|---|---|
| `palette-input` | headline-md 18/24 600 | `.title3` (15 pt) at 1.2×, regular | Palette and address input |
| `title` | headline-lg 24/32 600 | `.title2` | Sheet titles, empty states |
| `row-title` | body-md 14/20 | `.body` (13 pt) | Rows in palette, switcher, popovers |
| `row-detail` | label-lg 13/18 500 | `.subheadline` | Hosts, ages, counters |
| `caption` | label-sm 11/14 500 | `.caption1` | Hint bars, footers, section headers |
| `mono` | code-md 13/18 | SF Mono, `.body` size | URLs, command ids, payloads, `when` expressions |
| `mono-small` | code-sm 11/14 | SF Mono, `.caption1` size | Keycaps, link hints |

## Spacing and shape

| Token | Value | Use |
|---|---|---|
| `space-xs` … `space-xl` | 4 · 8 · 12 · 16 · 24 | Everything; no other values |
| `space-section` | 32 | Between groups in settings and sheets |
| `palette-padding` | 20 | Inner padding of E2 overlays |
| `keycap-gap` | 4 inside a chord, 8 between chords | Keycap groups |
| `radius-keycap` | 6 | Keycaps, link hints |
| `radius-row` | 10 | Selected rows, thumbnails' inner radius |
| `radius-card` | 16 | Thumbnails, cards in popovers |
| `radius-overlay` | 22 | E2 and E3 surfaces (continuous corners) |
| `radius-pill` | full | E1 indicators, toasts, origin chip, find bar |

Overlay widths from the mockups: address 640, command palette 720, answer panel 760, popovers 360,
switcher sized by its thumbnails (6 × 16:10). Palettes sit at 15 % from the top (address at 20 %).

## Elevation

| Level | Stitch approximation | Native | Shadow | Content dimming |
|---|---|---|---|---|
| E0 | none | the WebContents view | — | — |
| E1 | 16 px blur, inner border white 40 % / 12 % | `NSGlassEffectView` `.clear` | none | none |
| E2 | 28 px blur, 180 % saturation | `NSGlassEffectView` `.regular` in an `NSGlassEffectContainerView` | `0 20 48 -10` black 22 % + 1 px white ring 20 % | 10 % |
| E3 | 36 px blur | `.regular` with a tint | `0 32 72 -12` black 45 % + 1 px white ring 25 % | 30 % |

Rules: never glass on glass; one E2 or E3 at a time (an E3 collapses the E2 under it); glass only
where something floats above the page. On macOS 15 the fallback is `NSVisualEffectView`
(`.popover` for E2, `.hudWindow` for E3) with the same shadows and dimming.

## Motion

Timing curves map 1:1 onto `CAMediaTimingFunction(controlPoints:)`.

| Token | Duration | Curve | Use |
|---|---|---|---|
| `state-change` | 100 ms | ease-out | Selection, toggles, hover-free focus changes |
| `fade-fast` | 120 ms | ease-out | E1 in and out |
| `overlay-enter` | 200 ms | `(0.16, 1, 0.3, 1)`, scale 0.96 → 1 + fade | E2 |
| `sheet-enter` | 250 ms | `(0.16, 1, 0.3, 1)`, slide from the top edge | E3 |
| `origin-chip-hold` | 2.4 s | then `fade-fast` | Origin chip after navigation or tab switch |
| `toast-hold` | 3 s | then `fade-fast`; a click dismisses | Toasts |

Reduce Motion replaces scale and slide with a plain fade of the same duration.

## Voice

- Commands are imperative verbs: `Reopen Closed Tab`, `Copy Markdown Link`, `Toggle Reader Mode`.
- Shortcuts use the macOS glyphs `⌘ ⇧ ⌥ ⌃ ⏎ ⎋ ⇥`.
- Status chips are short and uppercase: `PROD`, `LOCALHOST:3000`, `AGENT ACTIVE`.
- Tone: discreet, a little cheeky, never crude. "Protection is on." "Wrapped and protected by
  default." "Quickie won't let bots spend recklessly." "No data leaves your Mac."
- Never claim what we cannot prove: no "100 % safe", no "undetectable". Numbers shown in the UI are
  measured, not decorative.
