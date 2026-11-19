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
| Tests | cargo test | `test` | yes | yes | Behavior |
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
| `ci.yml` | push to `main`, every PR | `mise run ci` on macOS (all gates, secret scan over the whole history); on PRs also `cargo mutants --in-diff` on Linux |
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
- UI snapshot tests arrive with the first `ui/*` crate.
- Branch protection on `main` (required checks, squash/rebase only) is set on GitHub after the
  first release.
