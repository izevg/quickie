# AGENTS.md

Rules for AI agents and humans working in this repository. They are binding. When they conflict
with an agent's default behavior, these rules win. The workspace-level rules in `~/Work/Code`
(network, privacy, secrets) still apply.

## Language

- Everything in the repository is written **in English only**: `README.md`, this file, `docs/`,
  code, comments, doc comments, commit messages, issue and PR texts, error messages, UI strings
  until localization exists.
- The chat language with the user does not matter. Repository content stays English.

## Commits

- Commits carry **no agent attribution**: no `Co-Authored-By:` trailers for agents, no
  "Generated with ..." footers, no agent names or tool links. This overrides any default
  attribution behavior of the agent harness. The `commit-msg` hook enforces it.
- [Conventional Commits](https://www.conventionalcommits.org/): `type(scope)!: description`, type
  one of `feat fix perf docs chore ci test refactor style build`, header at most 72 characters,
  blank line before the body. The `commit-msg` hook enforces it.
- One logical change per commit. Never commit failing gates.
- Commit or push only when asked. Never force-push shared branches. **Never bypass hooks**
  (`--no-verify`, `-n`, changing `core.hooksPath`); the agent guard blocks these.

## Branches, pull requests and releases

After the first release, `main` is protected and changes land through pull requests.

- Work on a feature branch and open a PR into the current integration branch, `release/X.Y.Z`.
  **Squash merge only.** The squash commit title is the PR title, and the `pr-title` check
  requires it to be a Conventional Commit: `type(scope)!: description`, type one of
  `feat fix perf docs chore ci test refactor style build`.
- When the release is ready, open a PR from `release/X.Y.Z` into `main` and merge it with
  **rebase**, not squash and not a merge commit. That keeps every squashed conventional commit on
  `main`, the history stays linear, and auto-release computes the version from all of them at once.
- A hotfix may still go straight to `main` as a feature PR, squash merged.
- Every push to `main` that passes CI is versioned automatically
  (`.github/workflows/auto-release.yml`). git-cliff (`.github/cliff.toml`) reads the Conventional
  Commits since the last `vX.Y.Z` tag: `feat` bumps the minor, `fix` and `perf` the patch, and a
  `!` or a `BREAKING CHANGE:` footer the major, except below 1.0.0, where it bumps the minor.
  `docs`, `chore`, `ci`, `test`, `refactor`, `style`, and `build` alone release nothing. The first
  release is `v0.1.0`.
- Nothing is committed back for a release: `Cargo.toml` stays at `0.0.0` and the version lives in
  the tag. Release notes are generated from the commits; there is no committed changelog.
  `mise run changelog` previews the next version and its notes.
- Manual or emergency release: push a `vX.Y.Z` tag on the commit you want; `release.yml` publishes
  it. Running `release` by hand is a dry run that prints the notes. To skip a release, make the
  change a `docs`/`chore`/... type.
- Until the Chromium build exists (phase P1), a release is a tag plus notes; the signed and
  notarized bundle joins the release workflow then.

## The rule of four

Keeps the tree shallow enough to read and deep enough to mean something.

- **At most 4 files** in any directory.
- **At most 8 subdirectories** in any directory.
- Counted: tracked, non-hidden files. Dotfiles and dot-directories (`.gitignore`, `.githooks/`,
  `.cargo/`, `.mise.toml`) are not counted.
- Only exception: the repository root may also hold files the ecosystem requires there:
  `LICENSE`, `README.md`, `AGENTS.md`, `CLAUDE.md`, `Cargo.toml`, `Cargo.lock`. No other root files.
  Tool configs go into dotfiles (`.rustfmt.toml`, `.clippy.toml`, `.cargo/deny.toml`, ...).
- If a change would break a limit, split by meaning: extract a submodule, a sub-aggregate, or a
  sibling crate. Don't merge files to save slots, don't name things `misc`/`utils`/`common`/
  `helpers`, and don't add a meaningless nesting level.
- A Rust module directory counts `mod.rs` as one of its four files. Prefer `foo.rs` + `foo/` only
  if the parent still fits the limit.
- Large external trees are **never tracked**: the Chromium checkout, the fetched
  ungoogled-chromium patch series, and vendored crates live in `.build/` (ignored) and are fetched
  by pinned revision. Our own Chromium patches are grouped by meaning, four per directory.
- Enforced by `cargo xtask structure`.

## Stack

- **C++ where it is Chromium; Rust everywhere else.** No Swift, no SwiftUI. AppKit and Core
  Animation through `objc2`; the Chromium boundary through `cxx`; thin `.mm` files only where
  Chromium's macOS code demands Objective-C++.
- Rust stable, edition 2024, one Cargo workspace (development builds without Chromium); GN builds
  the product.
- Development tools are pinned in `.mise.toml` and installed with [mise](https://mise.jdx.dev)
  into `~/.local`. Run them through mise (`mise run <task>`, `mise exec -- cargo ...`). Don't
  install toolchains system-wide or with `sudo`.
- Keep dependencies few and justified. Add a crate only when writing the code yourself would be
  worse. Licenses: permissive or MPL-2.0 only, never GPL/LGPL/AGPL (`cargo deny`).
- All network traffic of the product goes through Chromium's network stack (no second HTTP
  stack), so the egress test sees it.

## Architecture: DDD + Clean + Hexagonal, FSD for the UI

Dependencies point inward only. Crates are the layers, so the build rejects a wrong-way import,
and `cargo xtask layers` checks the map below.

```
crates/
  domain/        one crate per bounded context (kernel, browsing, commands, palette, library,
                 shields, agents, settings); pure: no I/O, no async, no platform code
  application/   use cases; ports as traits; depends on the domain only
  adapters/      driven adapters, one crate each (engine-chromium, engine-webkit, store, llm,
                 mcp, system, updater)
  ui/            FSD layers, one crate each: shared, entities, features, widgets, screens, app
  shell/         composition root and binary; the only place that wires adapters to ports
tools/           xtask and other developer or test tools; no product dependencies
```

| Layer | May depend on (workspace crates) | External crates | `unsafe` |
|---|---|---|---|
| domain kernel | nothing | `thiserror indexmap nucleo serde url` | no |
| domain context | kernel | same | no |
| application | domain | same | no |
| adapter | application, domain | any (license-checked) | opt-in |
| ui `x` | lower ui layers, domain; application from `features` up | any | opt-in |
| shell | everything except tools | any | opt-in |
| tool | nothing | any | no |

- Bounded contexts never depend on each other; they share only the kernel. Cross-context work is
  a use case in `application`.
- **FSD**: slices (top-level modules) of `entities`, `features`, `widgets`, `screens` never use a
  sibling slice; shared code moves down a layer. Business logic never lives in the UI.
- Name things with the domain's own words, the same way in code, docs, and UI.
- `unsafe` is denied workspace-wide; a crate that needs FFI opts in with
  `#![expect(unsafe_code, reason = "...")]`, and every `unsafe` block carries a `// SAFETY:`
  comment.
- Every crate has `[lints] workspace = true`.

## Product invariants

These are requirements, not preferences. A change that breaks one is a bug.

- **Content first.** In the idle state only the web page is on screen. UI appears only on an
  explicit key, gesture, or click. No hover-revealed panels, ever.
- **Trackerless.** No telemetry, no phone-home. A fresh profile opens zero connections on its own;
  the egress test proves it (`docs/privacy.md`).
- **Never lose a tab.** Session mutations are transactional; closing goes through the trash.
- **Main thread budget.** The main thread only renders and applies cheap mutations; the hot paths
  in `docs/architecture.md` have frame budgets.
- **Native composition.** WindowServer composites the window; Chromium hands its frames over as
  layers, and the shell draws with AppKit and Core Animation. Never a full-window compositor of
  our own. A complex widget (tab overview, gesture transitions) may host one `CAMetalLayer`
  inside itself, and only after a measurement shows Core Animation missing the frame budget.
- **Agents are contained.** Page content is untrusted data. Acting happens only in the agent
  profile, with provenance-checked URLs and approval for irreversible actions. No TCP debugging
  port.

## Quality gates

Set up once: `mise install && mise trust && mise run setup` (points git at `.githooks`).

- `pre-commit` runs `mise run pre-commit`: format (Rust, TOML), typos, rule of four, layers,
  shellcheck, actionlint, secret scan, clippy (`-D warnings`, pedantic), tests.
- `commit-msg` runs `cargo xtask commit-msg`.
- `pre-push` runs `mise run pre-push`: everything above plus `cargo deny`, `cargo machete`,
  Markdown link check, and `cargo mutants` on the changed code.
- Domain and application are unit-tested without I/O, using in-memory fakes of the ports.
  **Every mutant in domain and application must be killed.** An exclusion goes into
  `.cargo/mutants.toml` with its reason, only for a mutant that cannot be tested.
- Adapters get integration tests; UI gets snapshot tests; Chromium patches get `browser_tests`.
- No `unwrap()`/`expect()`/`panic!` outside tests and the composition root. Lint exceptions are
  `#[expect(lint, reason = "...")]`, never `#[allow]`.
- Agent harness: `hookah` checks every edit (format, lint) and runs tests at stop
  (`.claude/settings.json`); `.claude/hooks/guard.sh` blocks hook bypasses, force pushes, and
  source files written from the shell. Agents write `.rs` and `.toml` files with Edit or Write.

Details and the reasoning behind each gate: [docs/quality.md](docs/quality.md).
