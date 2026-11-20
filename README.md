<div align="center">

<img src="docs/assets/logo.png" width="180" alt="Quickie logo: a glass Q with a fast-forward sign, a lipstick mark, and a Do Not Disturb hanger">

# Quickie

**Fast. Discreet. Leaves no trace.**

The web, edge to edge; everything else is one keystroke away.
A personal, macOS-only web browser: Chromium underneath, a native Rust and AppKit shell on top.
The page fills the window, the interface answers the keyboard like VS Code, Liquid Glass appears
only where something floats, and nothing phones home.

[![CI](https://github.com/izevg/quickie/actions/workflows/ci.yml/badge.svg)](https://github.com/izevg/quickie/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/izevg/quickie?include_prereleases&sort=semver)](https://github.com/izevg/quickie/releases)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust edition 2024](https://img.shields.io/badge/rust-edition%202024-orange.svg)](Cargo.toml)
![Platform](https://img.shields.io/badge/platform-macOS%2026-lightgrey.svg)
![Engine](https://img.shields.io/badge/engine-Chromium-4285F4.svg)
![Telemetry: none](https://img.shields.io/badge/telemetry-none-brightgreen.svg)

</div>

> **Status: pre-alpha.** The design and the quality harness are in place; there is no browser
> code yet. Next is phase P0: the shell on a WebKit stand-in, so the interface can be lived in
> before the first Chromium build. See the [roadmap](#roadmap).

## Why

Every browser picks a compromise this one refuses:

- **Arc** proved a browser can be rethought, then sank under its own weight: "bloated", a
  SwiftUI and TCA stack its makers abandoned, tabs lost to sync, and a shutdown on 2026-12-22.
- **Vivaldi** hides its panels behind hover zones that fire when you don't want them and miss
  when you do.
- **Mainstream browsers** ship telemetry, field trials, and search suggestions that send every
  keystroke with a persistent ID.
- **Agentic browsers** let page content steer the agent with your logins attached.

Quickie keeps the good idea (the page is the app, the keyboard is the interface) and drops the
rest. In the idle state you see only the site. Everything else appears on a keystroke, floats on
glass above the page, and is gone when you are done:

```text
┌──────────────────────────────────────────────────────────────┐
│                                                              │
│                 the page, edge to edge                       │
│        no tab bar · no toolbar · no sidebar · no hover       │
│                                                              │
└──────────────────────────────────────────────────────────────┘
                              ⌘⇧P
┌──────────────────────────────────────────────────────────────┐
│          ╭────────────────────────────────────────╮          │
│          │ > reo                                  │          │
│          │   Reopen closed tab            ⌘ ⇧ T   │          │
│          │   Restore last session                 │          │
│          │   > commands · @ headings · ? agent    │          │
│          ╰────────────────────────────────────────╯          │
└──────────────────────────────────────────────────────────────┘
```

## What it does

| Area | What you get | Phase |
|---|---|---|
| Content first | Full-bleed pages; no tab bar, toolbar, or sidebar; nothing revealed on hover | P0 |
| Command palette | `⌘⇧P` for every command, `⌘P` for tabs and history, fuzzy search, frecency, prefixes `>` `@` `#` `?` `!` | P0 |
| Smart tab switching | `⌃⇥` in most-recently-used order: a quick tap flips back, a hold shows thumbnails you can filter by typing | P0 |
| Keymap | VS Code-style bindings with `when` contexts and chords, all rebindable; works on non-Latin layouts | P0 |
| Gestures | Native two-finger back and forward with a live preview, pinch for a tab overview | P0 |
| Liquid Glass with elevation | Four levels (page, indicators, overlays, sheets); glass only where something floats, never glass on glass | P0 |
| Chromium engine | Web compatibility, DevTools, MV3 extensions | P1 |
| Trackerless | ungoogled-chromium base, built-in ad and tracker blocking, an egress test that proves zero phone-home | P2 |
| Built-in agent | `?` in the palette asks any OpenAI-compatible endpoint (Ollama, LM Studio, hosted APIs) about the page | P4 |
| External agents | MCP over a Unix socket; actions only in an isolated agent profile, with approvals | P4 |
| Anti-fingerprinting | Per-site, per-session noise for canvas, audio, and WebGL | P5 |

### Default keys

All of them are commands in the palette and can be rebound in the keymap editor.

| Keys | Command |
|---|---|
| `⌘⇧P` / `⌘K` | Command palette |
| `⌘P` | Go to tab, bookmark, or history |
| `⌘L` | Address overlay (current URL, site actions) |
| `⌘T` | New tab (opens the address overlay) |
| `⌘⇧N` | New tab in a throwaway jar |
| `⌃⇥` / `⌃⇧⇥` | Next / previous tab in most-recently-used order |
| `⌘1` … `⌘9` | Pinned slots |
| `⌘⌥1` … `⌘⌥9` | Pin the tab to a slot |
| `⌘[` / `⌘]` | Back / forward |
| `⌘R` | Reload |
| `⌘D` | Bookmark the page |
| `⌘F` | Find in page |
| `⌘G` / `⌘⇧G` | Next / previous match |
| `F` / `⇧F` | Link hints (when no text field has focus); `⇧F` opens the link in a background tab |
| `⌘⇧\` | Tab overview (or pinch in on the page) |
| `⌘⇧C` | Copy the URL |
| `⌘⌥C` | Copy as Markdown link |
| `⌘⇧T` | Reopen the closed tab |
| `⌘W` | Close the tab (it goes to the trash, not away) |
| `⌘J` | Downloads |
| `⌘⌥S` | Shields for the site |
| `⌘N` / `⌘⇧W` | New window / close the window (its tabs go to the trash) |

## Privacy

Trackerless here means a fresh profile opens **zero** connections on its own, and the build proves
it: the egress test starts the browser with a clean profile, records the netlog, and fails on any
host that is not on the allowlist.

| Chrome sends | Quickie |
|---|---|
| Usage metrics, crash reports, field trials | Removed at build time; crash dumps stay on disk |
| Search suggestions with every keystroke | Off |
| Safe Browsing lookups, component updates | Off, or through your own server |
| Sync, Google sign-in, Translate, Privacy Sandbox | Removed |
| Gemini and other AI features | Removed; the built-in agent talks only to the endpoint you configure, only when you ask |

Details, the full checklist, and what breaks: [docs/privacy.md](docs/privacy.md).

## How it is built

```
ui (Feature-Sliced Design) ──▶ application (use cases, port traits) ──▶ domain (pure)
                                     ▲
shell (composition root) ──▶ adapters: Chromium (cxx) · WebKit stand-in · SQLite · LLM · MCP · Keychain
```

- **C++ where it is Chromium; Rust everywhere else.** AppKit and Core Animation through `objc2`,
  the Chromium boundary through `cxx`. No Swift, no SwiftUI, no HTML chrome.
- **DDD, hexagonal, clean.** Each bounded context is a crate; the build rejects a wrong-way import.
- **Small diff against Chromium.** ungoogled-chromium patches plus our own, on the Extended Stable
  channel, rebased with agent help.
- **One network stack.** Every request, LLM calls included, goes through Chromium, so the egress
  test sees everything.

The whole picture (C1 to C4, crates, hot-path budgets, security): [docs/architecture.md](docs/architecture.md).

## Roadmap

| Phase | Result | Done when |
|---|---|---|
| P0 | The shell on a WebKit stand-in: palette, switcher, glass, gestures, keymap | It works as a daily browser for a week, without extensions |
| P1 | Chromium with ungoogled patches, the native window, the `cxx` bridge | The same UI on Chromium, with DevTools and extensions |
| P2 | Shields, privacy patches, egress test in CI | A fresh profile reaches no unsolicited host in 10 minutes |
| P3 | Native sheets instead of Chromium bubbles, link hints, tab overview, keymap editor | — |
| P4 | Built-in and external agents, approvals, audit log | `?` answers through Ollama; an external agent drives a tab in the agent profile |
| P5 | Anti-fingerprinting, self-hosted services | privacytests.org results on par with Brave |

## Development

Requirements: macOS on Apple Silicon and [mise](https://mise.jdx.dev). Every other tool is pinned
in `.mise.toml` and installed into `~/.local`.

```sh
git clone git@github.com:izevg/quickie.git && cd quickie
mise install && mise trust && mise run setup   # tools and git hooks
mise run pre-commit                            # the fast gates
mise run pre-push                              # everything, mutation testing included
```

Running the development app (on the `WebKit` engine until the Chromium build exists):

```sh
mise exec -- cargo run -p quickie                  # the bare binary; URLs or files may follow `--`
mise run bundle                                    # target/debug/Quickie.app
mise run bundle --release                          # target/release/Quickie.app
open target/debug/Quickie.app
open -a target/debug/Quickie.app https://example.com page.html
```

`mise run bundle` wraps `cargo xtask bundle`: it builds the shell, lays out `Quickie.app` with its
`Info.plist` (bundle id `dev.quickie.Quickie`, `--bundle-id` for a throwaway build), and an icon
made from `docs/assets/logo.png` at bundle time, and signs it ad hoc so WebKit's helper processes
launch. As a bundle, Quickie opens the links and files other apps hand it, and the palette's
**Set as Default Browser** asks macOS to make it the default browser (macOS confirms with you).
The ad-hoc signature is for development only: the Developer ID signed and notarized build comes
with the Chromium product build in P1.

| Gate | Runs |
|---|---|
| pre-commit | rustfmt, taplo, typos, the rule of four, the layer map, shellcheck, actionlint, secret scan, clippy (pedantic, warnings are errors), tests |
| commit-msg | Conventional Commits, no agent attribution |
| pre-push | all of the above, cargo-deny (no GPL family), unused dependencies, Markdown links, cargo-mutants on domain and application |
| CI | the same tasks on macOS; on pull requests also mutation testing of the changed code |

Work happens on feature branches, squash merged into `release/X.Y.Z`; a release branch is rebase
merged into `main`, where git-cliff computes the version and the release notes from the commits.

## Documentation

| Page | Covers |
|---|---|
| [AGENTS.md](AGENTS.md) | Binding rules for humans and agents: language, commits, branches, the rule of four, layers, invariants, gates |
| [Architecture](docs/architecture.md) | Stack, C1 to C4, crates and ports, Liquid Glass elevation, engine sync, repository layout, infrastructure, roadmap |
| [Quality](docs/quality.md) | Gates, hooks, lints, the layer map, mutation testing, CI and releases |
| [Privacy](docs/privacy.md) | The trackerless model, the Chromium services checklist, the egress test |
| [Engine](engine/README.md) | Fetching, patching and building Chromium with `//quickie`; what is verified, what is `TODO(verify)` |
| [Lessons](docs/lessons.md) | What went wrong in Arc, Dia, Vivaldi, Orion, Zen, Helium, and agentic browsers, and the rule each lesson became |
| [Design system](docs/design/system.md) | Color, type, spacing, elevation, motion, and voice, mapped onto native macOS |
| [Component inventory](docs/design/inventory.md) | Every mocked screen and component, cut into FSD layers |
| [Design ideas](docs/design/ideas.md) | What the mockups proposed, and what we adopt, defer, or reject |
| [Stitch prompt](docs/design/stitch-prompt.md) | The prompt behind the [mockups](docs/design/mocks/) |
| [Releases](https://github.com/izevg/quickie/releases) | Every release, with notes generated from Conventional Commits |

## Contributing

Read [AGENTS.md](AGENTS.md) first. Quickie is built for one person, but the rules hold for anyone,
human or agent: English only, Conventional Commits without agent attribution, the rule of four for
the file tree, the layer map, and the quality gates. Hooks are never bypassed.

## License

[Apache License 2.0](LICENSE). Chromium and ungoogled-chromium, which Quickie builds on, are
BSD-3-Clause and keep their own notices; every dependency must stay license-compatible
(`cargo deny` rejects the GPL family).
