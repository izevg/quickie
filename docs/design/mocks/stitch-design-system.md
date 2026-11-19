---
name: "Quickie Liquid Glass Design System"
description: "A keyboard-first, content-immersive design system for Quickie personal desktop browser on macOS Tahoe featuring strict Liquid Glass elevation tiers."
---

## Overview

Quickie redefines personal desktop web browsing on macOS Tahoe by making content completely sovereign. In its idle state, the active website claims the entire 1512x982 display edge-to-edge without title bars, tabs, toolbars, or window controls. Interface appears exclusively on deliberate keyboard shortcuts or gestures, manifesting as Apple Liquid Glass elevated translucent surfaces that float weightlessly above the live DOM.

The visual language follows strict optical elevation tiers:
- **E0 (Web Content):** Pure zero-elevation canvas rendering realistic web environments (news editorial, code repositories, documentation). No glass.
- **E1 (Transient Micro-Indicators):** Ultra-thin 2px edge progress bars, compact origin indicators, and micro-toasts floating with clear glass and zero cast shadow.
- **E2 (Floating Overlays):** Interactive command palettes, fuzzy tab switchers, and find bars rendered in classic Apple Liquid Glass (20–24px smooth corner radii, soft multi-stage specular borders, ambient medium drop shadows, with content dimmed 10%).
- **E3 (Security & Permissions):** Denser tinted sheets with heavy depth shadows and 30% background scrim for security-critical decisions.

## Colors

The palette employs Apple system neutrals with a vibrant System Blue primary accent, Semantic Red for destructive or permission revocations, and Amber for autonomous agent activities.

- **Primary:** Apple System Blue for selected states, focus rings, and primary execution.
- **System Red:** Security warnings, destructive actions, and permission denials.
- **System Amber:** AI/Agent status indicators, synthesis operations, and background tasks.

```yaml
colors:
  accent: "#0071E3"
  primary: "#0071E3"
  on-primary: "#FFFFFF"
  destructive: "#FF3B30"
  on-destructive: "#212121"
  agent: "#FF9F0A"
  on-agent: "#000000"
  success: "#34C759"
  on-success: "#424242"
  content-bg-light: "#FBFBFD"
  content-text-light: "#1D1D1F"
  content-subtext-light: "#6E6E73"
  content-bg-dark: "#0D1117"
  content-card-dark: "#161B22"
  content-text-dark: "#C9D1D9"
  content-subtext-dark: "#8B949E"
  content-border-dark: "#30363D"
  syntax-keyword: "#FF7B72"
  syntax-entity: "#79C0FF"
  syntax-string: "#A5D6FF"
  light:
    primary: "#0071E3"
    on-primary: "#FFFFFF"
    surface: "#FFFFFF"
    on-surface: "#1D1D1F"
    surface-container: "#F5F5F7"
    on-surface-container: "#1D1D1F"
    surface-container-high: "#E8E8ED"
    outline: "#86868B"
    outline-variant: "#D2D2D7"
  dark:
    primary: "#0A84FF"
    on-primary: "#1F1F1F"
    surface: "#1C1C1E"
    on-surface: "#F5F5F7"
    surface-container: "#2C2C2E"
    on-surface-container: "#F5F5F7"
    surface-container-high: "#3A3A3C"
    outline: "#8E8E93"
    outline-variant: "#48484A"
```

## Typography

Quickie uses **Geist** for crisp Apple-style UI hierarchy, and **JetBrains Mono** for URLs, keyboard shortcuts, code blocks, and system keycaps.

```yaml
typography:
  display-lg:
    fontFamily: "Geist"
    fontSize: 36px
    fontWeight: 700
    lineHeight: 44px
    letterSpacing: -0.5px
  display-lg-mobile:
    fontFamily: "Geist"
    fontSize: 28px
    fontWeight: 700
    lineHeight: 36px
    letterSpacing: -0.25px
  headline-lg:
    fontFamily: "Geist"
    fontSize: 24px
    fontWeight: 600
    lineHeight: 32px
    letterSpacing: -0.3px
  headline-lg-mobile:
    fontFamily: "Geist"
    fontSize: 20px
    fontWeight: 600
    lineHeight: 28px
    letterSpacing: -0.2px
  headline-md:
    fontFamily: "Geist"
    fontSize: 18px
    fontWeight: 600
    lineHeight: 24px
  body-lg:
    fontFamily: "Geist"
    fontSize: 16px
    fontWeight: 400
    lineHeight: 24px
  body-md:
    fontFamily: "Geist"
    fontSize: 14px
    fontWeight: 400
    lineHeight: 20px
  label-lg:
    fontFamily: "Geist"
    fontSize: 13px
    fontWeight: 500
    lineHeight: 18px
  label-sm:
    fontFamily: "Geist"
    fontSize: 11px
    fontWeight: 500
    lineHeight: 14px
  code-md:
    fontFamily: "JetBrains Mono"
    fontSize: 13px
    fontWeight: 500
    lineHeight: 18px
  code-sm:
    fontFamily: "JetBrains Mono"
    fontSize: 11px
    fontWeight: 500
    lineHeight: 14px
```

## Layout & Spacing

Window dimensions strictly adhere to 1512x982 (MacBook Pro 14" default display scaling). Layouts utilize an absolute floating layering model where content spans 100vw/100vh with no internal frames, while command interfaces float centered horizontally and vertically or tethered to specific top optical bounds (e.g. 15% top margin for command palettes).

```yaml
spacing:
  gutter-xs: 4px
  gutter-sm: 8px
  gutter-md: 12px
  gutter-lg: 16px
  gutter-xl: 24px
  palette-padding: 20px
  keycap-gap: 8px
  section: 32px
  page-margin: 32px
```

## Elevation & Depth

Elevation strictly dictates glass thickness, backdrop filtration, and cast shadows:
- **E0 (Web Surface):** No glass, opacity 100%, z-index 0.
- **E1 (Micro Indicators):** 16px backdrop blur, subtle inner border `rgba(255,255,255,0.4)` (light) / `rgba(255,255,255,0.12)` (dark), 0px drop shadow, z-index 20.
- **E2 (Interactive Overlays):** 28px backdrop blur, 180% saturation boost, soft double shadow (`0 20px 48px -10px rgba(0,0,0,0.22), 0 0 0 1px rgba(255,255,255,0.2)`), dimming background by 10%, z-index 40.
- **E3 (Security Sheets):** 36px backdrop blur, heavy modal shadow (`0 32px 72px -12px rgba(0,0,0,0.45), 0 0 0 1px rgba(255,255,255,0.25)`), dimming background by 30%, z-index 50.

Rule: Never stack glass on glass. Only one E2 or E3 overlay may be open at a given moment.

## Shapes

Organic Apple continuous squircle-like curves. Overlays use a signature 22px border radius, keycaps use 6px radius, and micro-toasts use full pill styling.

```yaml
rounded:
  sm: 4px
  keycap: 6px
  md: 10px
  lg: 16px
  overlay: 22px
  full: 9999px
```

## Motion & Micro-interaction

Motion provides instant tactile reassurance without frivolous bounce:
- E2 overlays enter with a subtle spring scale `0.96 -> 1.0` combined with rapid fade `(200ms cubic-bezier(0.16, 1, 0.3, 1))`.
- E3 modal sheets slide up smoothly with `(250ms cubic-bezier(0.16, 1, 0.3, 1))`.
- Micro-indicators fade with `120ms ease-out`.

```yaml
motion:
  state-change:
    duration: 100ms
    delay: 0ms
    timingFunction: ease-out
  overlay-enter:
    duration: 200ms
    delay: 0ms
    timingFunction: cubic-bezier(0.16, 1, 0.3, 1)
  sheet-enter:
    duration: 250ms
    delay: 0ms
    timingFunction: cubic-bezier(0.16, 1, 0.3, 1)
  fade-fast:
    duration: 120ms
    delay: 0ms
    timingFunction: ease-out
```

## Voice & Content Strategy

Command-first, concise, and technically respectful:
- Action names are imperative verbs (`Search Tabs`, `Toggle Reader Mode`, `Copy Markdown Link`, `Inspect Network Requests`).
- Shortcuts are displayed using authentic macOS glyph symbols (`⌘`, `⇧`, `⌥`, `⌃`, `⏎`, `⎋`).
- Status chips provide immediate context (`PROD`, `LOCALHOST:3000`, `SSL VALID`, `AGENT ACTIVE`).

## Components

- `components/button.html`
- `components/card.html`
- `components/modal.html`
- `components/searchbar.html`
- `components/badge.html`
- `components/keycap.html`
- `components/origin-chip.html`
- `components/command-row.html`
- `components/toast-pill.html`
