# Quality harness

Guardrails that make a bad change fail early and loudly, whether a human or an agent wrote it.
The binding rules are in [AGENTS.md](../AGENTS.md); this page explains how they are enforced.

## Setup

```sh
mise install          # pinned tools from .mise.toml into ~/.local
mise trust            # allow this repository's mise config
mise run setup        # git config core.hooksPath .githooks
```

The hooks call `~/.local/bin/mise`; set `MISE` to use another path. A missing mise fails the hook
instead of skipping it.

## Gates

| Gate | Tool | `mise run` | pre-commit | pre-push | What it protects |
|---|---|---|---|---|---|
| Rust format | rustfmt | `fmt` | yes | yes | Diff noise |
| TOML format | taplo | `toml` | yes | yes | Diff noise in manifests and configs |
| Spelling | typos | `typos` | yes | yes | Code and docs |
| Rule of four | `cargo xtask structure` | `structure` | yes | yes | A tree people and agents can read |
| Layer map | `cargo xtask layers` | `layers` | yes | yes | DDD/hexagonal direction, pure core, FSD slices, unsafe policy, workspace lints |
| Shell scripts | shellcheck | `shell` | yes | yes | Git and agent hooks |
| Secrets | gitleaks (staged) | `secrets` | yes | yes | Keys and tokens never enter history |
| Lint | clippy `-D warnings`, pedantic + restrictions | `lint` | yes | yes | Bugs, panics, unstated exceptions |
| Tests | cargo test | `test` | yes | yes | Behavior, including the UI stories' plays ([UI tests](#ui-tests)) |
| UI stories | `cargo xtask ui check` | `ui` | — | — | Widget looks against reference images; strict in CI ([UI tests](#ui-tests)) |
| Dependencies | cargo-deny | `deny` | — | yes | Licenses (no GPL family), advisories, sources |
| Unused deps | cargo-machete | `machete` | — | yes | Dependency creep |
| Links | lychee `--offline` | `links` | — | yes | Internal Markdown links and anchors |
| Mutation | cargo-mutants | `mutants` | — | yes | Tests that really check domain and application |
| Workflows | actionlint | `actions` | yes | yes | GitHub Actions syntax and shell |
| Commit message | `cargo xtask commit-msg` | — | commit-msg hook | — | Conventional Commits, no agent attribution |

`mise run pre-commit` and `mise run pre-push` run the bundles by hand. The tasks are the single
source of truth for both hooks and CI.

## CI and releases (GitHub Actions)

| Workflow | When | What |
|---|---|---|
| `ci.yml` | push to `main`, every PR | `mise run ci` on macOS (all gates, secret scan over the whole history); `mise run conformance` on macOS (the engine conformance suite on `WKWebView`); `mise run ui` on macOS, strict, with the renders and diffs as the `ui-renders` artifact on failure; on PRs also `cargo mutants --in-diff` on Linux |
| `pr-title.yml` | every PR | The title is a Conventional Commit (it becomes the squash commit) |
| `auto-release.yml` | `ci` succeeded on a push to `main` | git-cliff computes the next version; nothing releasable ends the run |
| `release.yml` | called by auto-release, a pushed `v*` tag, or by hand (dry run) | Tag + GitHub Release with notes from git-cliff |

Actions are pinned by commit SHA. The branch and merge rules are in [AGENTS.md](../AGENTS.md).

## Lints

- Workspace lints in `Cargo.toml`; every crate inherits them (`[lints] workspace = true`, checked).
- `clippy::pedantic` as warnings, which `-D warnings` turns into errors.
- Denied: `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`,
  `print_stdout`, `print_stderr`, `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`.
  Tests may unwrap, expect, panic and print (`.clippy.toml`).
- `allow_attributes` and `allow_attributes_without_reason` are denied: an exception is
  `#[expect(lint, reason = "...")]`, which also fails once the exception is no longer needed.
- `unsafe_code` is denied; only adapters, UI and shell crates may opt in, and `cargo xtask
  layers` rejects an opt-in anywhere else.

## Layer map

`cargo xtask layers` reads `cargo metadata` and checks every workspace crate:

- its directory is on the map (`crates/domain/<context>`, `crates/application`,
  `crates/adapters/<name>`, `crates/ui/<fsd layer>`, `crates/shell`, `tools/<name>`);
- its normal dependencies on workspace crates follow the table in AGENTS.md (dev-dependencies are
  free);
- domain and application use only the allowed external crates;
- it inherits the workspace lints and opts into `unsafe_code` only where allowed;
- in the sliced UI layers, no slice uses a sibling slice through `crate::` or `super::`; in
  `widgets`, whose slices are grouped by meaning, a slice is `group/slice`, so two slices of one
  group are siblings too.

The table lives in `tools/xtask/src/layers/policy.rs`; change it together with AGENTS.md.

## Mutation testing

- Scope: `crates/domain/**` and `crates/application/**` (`.cargo/mutants.toml`). Adapters and UI
  are covered by integration and snapshot tests instead.
- Policy: every mutant is caught or the push fails. An exclusion needs an `exclude_re` entry with
  the reason, and only for a mutant that cannot be tested.
- With an upstream branch, only the changed code is mutated (`--in-diff`); without one, the whole
  scope runs. `--iterate` reuses earlier results.
- The engine conformance suite (`application/src/engine/conformance`) is application code, so its
  checks are mutated too: they run against the in-memory reference engine, and against broken
  variants of it that each check must fail. The WebKit adapter runs the same suite in
  `mise run conformance`, outside mutation.

## UI tests

Every widget and the Settings panes have **stories**: named states built from fixtures, the way
Storybook has them, but native and headless. They exist so that anyone without a display (an
agent, CI) can see a widget in seconds, and so that a change in looks or behaviour fails a test.

- **Where.** The harness is `quickie-ui-shared`'s `stories` feature
  (`crates/ui/shared/src/stories/`): the lowest UI layer, so `widgets` and `screens` both use it
  and the layer map does not change. Only dev-dependencies turn the feature on; the product never
  compiles it. Each UI crate lists its stories in a `stories` test target without the libtest
  harness (`crates/ui/<layer>/tests/stories/`), so `AppKit` runs on the main thread. A story
  uses only the crate's public API.
- **A story** is a group, a widget, a state, the themes it is drawn in (light, dark, Increase
  Contrast light and dark), a window size, and a function that builds the state in an offscreen
  window and plays it. States cover the empty, error, and loading cases, long text, Russian and
  English strings, and narrow windows.
- **Renders.** `cargo xtask ui render [<widget>[/<state>[.<theme>]]] [--scale 2]` writes
  `target/ui/<widget>/<state>.<theme>.png`, cropped to the widget plus 24 pt, and
  `target/ui/<widget>/<state>.accessibility.txt`, the accessibility tree. `cargo xtask ui list`
  names every story. No engine, no network, no display: the window is never ordered in, and the
  view draws itself into a bitmap (`cacheDisplayInRect:toBitmapImageRep:`). A render takes
  1–50 ms; a whole crate about a second after the build.
- **Glass stand-in.** In a story, `GlassSurface` is an opaque fill in the window background
  colour (accent-tinted for E3) instead of Liquid Glass or the visual effect view. Neither
  draws offscreen (glass renders empty, and translucent content inside it draws twice), and the
  stand-in makes a render independent of what is behind it. The page under a widget is a fixed
  grey. Shadows are layer shadows and do not draw into the bitmap, and controls draw as in an
  inactive window (a switch that is on is grey, not accent). Renders show layout, text, colour,
  and state; how the glass itself looks needs a screen.
- **Plays.** A story can press keys and click: `scene.type_text("rel")`,
  `scene.press(Key::Down)`, `scene.press_with(Key::Return, Modifiers::SHIFT)`,
  `scene.click(&view)`, with `scene.view_labelled("…")` to find a control by its accessibility
  title or label. Keys are real `NSEvent`s sent to the window, named by their US key codes, so
  a text field's editor and delegate react as on screen; a click on a control goes through
  `performClick:`, since mouse events do not reach a control's tracking in a window that was
  never key. Keys the browser routes before the window sees them (`↓` in the palette, `⏎` in
  the find bar) go through `scene.route`, which does what the shell does with them. Checks
  (`scene.check`, `scene.check_eq`, `scene.accessibility()`) fail the story.
- **Visual tests.** `cargo test` runs every story in `check` mode: render, play, and compare
  with the reference `crates/ui/<layer>/tests/snapshots/<group>/<widget>/<state>/<theme>.png`
  (one directory per state, one file per theme, which keeps the rule of four). On a mismatch it
  writes `target/ui/<widget>/<state>.<theme>.diff.png` (differing pixels red, pixels that only
  moved by one yellow, over a faded reference) beside the render.
- **Tolerance.** Two pixels match when their YIQ difference is at most 0.1 (the `pixelmatch`
  measure). A differing pixel is forgiven when each image has a matching pixel within one pixel
  of the other's: anti-aliasing and hinting move edges by a pixel. At most 0.01 % of the pixels
  (never fewer than 8) may differ beyond that, so a changed digit still fails. A changed size
  always fails.
- **1×.** References are drawn at one pixel per point: a quarter of the bytes of 2×, and enough
  for layout, text, and colour. `render --scale 2` draws 2× for a closer look.
- **CI is the source of truth.** Text rasterizes differently across macOS versions, so the
  references record the renderer they come from (`tests/snapshots/.renderer`, `macos-<major>`).
  Where it differs from the machine running the tests, the stories render and play but are not
  compared, and say so. The `ui stories` job runs strict (`QUICKIE_UI_STRICT=1`): there a
  missing reference or another renderer fails, and the renders are uploaded as `ui-renders`.
- **Accepting references**, only on purpose: download the `ui-renders` artifact of the failing
  run and `cargo xtask ui accept --from <dir> [<widget>[/<state>]]`, then look at every changed
  PNG before committing it. `cargo xtask ui accept [<filter>]` without `--from` takes local
  renders, and is refused for a subset when the references come from another renderer.

## Agent harness

- `.claude/settings.json` runs `hookah` (installed in `~/.local/bin`) through mise: format and lint after
  each edit, tests at stop, mutation when tests change. It only runs the tools this repository
  configures.
- `.claude/hooks/guard.sh` (PreToolUse, Bash) blocks `--no-verify`, `git commit -n`, changing
  `core.hooksPath`, and `git push --force`/`-f`. It also blocks writing `.rs` and `.toml` files
  from the shell (`>`, `tee`, `sed -i`): hookah checks only files written with Edit or Write.
  Anything the guard blocks, a human runs by hand after a decision.

## Not yet covered

- Engine gates (GN build, `browser_tests`, egress test, signing) arrive with the Chromium checkout
  in phase P1/P2. A Chromium build does not fit hosted runners; it needs the self-hosted build
  machine from [architecture.md](architecture.md), §7.
- Branch protection on `main` (required checks, squash/rebase only) is set on GitHub after the
  first release.
