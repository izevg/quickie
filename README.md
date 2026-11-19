# Quickie

A personal, macOS-only web browser: a Chromium fork with a native Rust/AppKit shell.
Full-bleed content, no hover-revealed panels, VS Code-style keyboard control, Liquid Glass with
real elevation, trackerless by default, a built-in agent on any OpenAI-compatible endpoint, and
MCP for external agents.

| Document | About |
|---|---|
| [AGENTS.md](AGENTS.md) | Binding rules: language, commits, the rule of four, layers, invariants, gates |
| [docs/architecture.md](docs/architecture.md) | C1–C4, stack and crates, DDD/hexagonal/clean + FSD, repository layout, infrastructure, roadmap |
| [docs/quality.md](docs/quality.md) | The quality harness: gates, hooks, lints, layer map, mutation testing |
| [docs/lessons.md](docs/lessons.md) | Other browsers' mistakes turned into our rules, with sources |
| [docs/privacy.md](docs/privacy.md) | Trackerless: what we remove, what replaces it, how we prove it |
| [docs/design/stitch-prompt.md](docs/design/stitch-prompt.md) | Google Stitch prompt for reference mockups, tagged for FSD slicing |

## Getting started

```sh
mise install && mise trust && mise run setup   # tools and git hooks
mise run pre-commit                            # the fast gates
```

## Key decisions

1. **Engine: Chromium.** WebKit is out (no CDP for agents, weak inspector). Our own engine is out
   (years and tens of millions).
2. **Languages: C++ where it is Chromium; Rust everywhere else.** No Swift/SwiftUI. AppKit and
   Core Animation through `objc2`, the Chromium boundary through `cxx`.
3. **One build system: GN**, with Rust built in-tree as Chromium supports it. A Cargo workspace
   builds the shell against a WebKit stand-in for phase P0, without a Chromium build.
4. **Base: ungoogled-chromium patches** (BSD-3) plus our own, Brave-style (file overrides before
   patches). Branch: **Extended Stable** (a rebase every 8 weeks).
5. **Application logic: DDD + hexagonal + clean.** A bounded context is a crate, a port is a
   trait; the build and `cargo xtask layers` enforce the direction.
6. **UI: Feature-Sliced Design** as crates `ui/{shared,entities,features,widgets,screens,app}`.
7. **Provably trackerless:** an egress test on a fresh profile; all traffic, LLM calls included,
   goes through Chromium's network stack and shows up in the netlog.
8. **Agents:** the built-in one talks to any OpenAI-compatible endpoint (Ollama and others), no
   Apple Foundation Models; external ones connect over MCP on a Unix socket. Acting happens only
   in an isolated agent profile with approvals. No TCP debugging port, ever.
