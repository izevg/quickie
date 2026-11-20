# Engine

Quickie's engine is our fork of WebKit, patched and built by us
([architecture.md](../docs/architecture.md), "Decision: our own WebKit"). Until the fork ships,
both profiles run on the system `WKWebView` through `engine-webkit`.

| Path | What |
|---|---|
| [webkit/](webkit/) | The fork's build (active) |
| [chromium/](chromium/) | The Chromium scaffold, parked as reference ([chromium/README.md](chromium/README.md)) |

This page lives in `engine/` rather than `docs/` because `docs/` already holds its four files
(the [rule of four](../AGENTS.md#the-rule-of-four)).
