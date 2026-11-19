# Trackerless: model and checklist

**Definition.** On a fresh profile with no user action, the browser opens **no** network
connections. After user actions, it connects only to hosts the user opened and to services the
user explicitly enabled. This is checked automatically (§4), not promised.

## 1. Base

- **ungoogled-chromium** (BSD-3) works on three levels: patches, binary pruning, and **domain
  substitution** (Google domains in the source become the nonexistent `qjz9zk`, and
  `block-trk-and-subdomains.patch` blocks them as a safety net for missed calls).
- The **Helium** pattern: own self-hosted services instead of Google (extension proxy, filter
  lists, updates). Helium is GPL-3.0: copy its patches only with that license in mind; the
  ungoogled-chromium and Brave approaches are safe to reuse.
- Not a base: **Thorium** (Google API keys baked in for Sync) and **Brave** as a whole (P3A
  analytics on by default).

## 2. Chromium services checklist

| Service | In Chrome | In Quickie | What breaks |
|---|---|---|---|
| UMA/UKM, URL-keyed collection | on / opt-in | removed at build time | nothing |
| Crash reporting | opt-in | dumps stay local | — |
| Variations / field trials | on | `disable-fetching-field-trials` | nothing |
| GCM / Web Push | on | `disable-gcm` | Web Push |
| Safe Browsing | on | off; phishing and malware lists in Shields; optional proxy through quickie-services | weaker anti-phishing |
| Component updater (CRLSet, ...) | on | own server (after Brave's `go-update`) or off | no fresh CRLSets |
| Widevine | on demand | **opt-in**, direct to Google; stay within 12 months of current Chromium | Netflix/Spotify need it |
| Chrome Web Store / extension updates | on | CRX proxy through quickie-services, or manual installs | extension auto-updates |
| Spellcheck dictionaries | gvt1 | bundled | — |
| Translate | on | off (`toggle-translation-via-switch`) | translation |
| Sync / GAIA | offered | `disable-gaia`, no API keys | no sync (not wanted) |
| RLZ, network time, domain reliability, MEI preload, intranet redirect detector, DIAL | on | the matching ungoogled patches | nothing |
| Privacy Sandbox / Topics | on | `disable-privacy-sandbox` | nothing |
| Gemini / Chrome AI features | on **[unverified]** | `disable-ai` | — |
| Our built-in agent | — | connects only to the configured OpenAI-compatible endpoint, only on an explicit `?` request; no endpoint by default; goes through Chromium's network stack, so it shows in the netlog | — |
| Search suggestions | on | **off** (Chrome sends typed input with a persistent ID; Leith/TCD study) | no suggestions |
| Optimization Guide | on | off | — |
| Fonts googleapis, WebRTC log uploader, Web Store URLs | on | ungoogled patches | — |

## 3. Protection beyond "no phone-home"

| Mechanism | Implementation | Phase |
|---|---|---|
| Tracker and ad blocking | `adblock-rust` in the network service: network rules + cosmetic + scriptlets (uBO syntax); lists: EasyList, EasyPrivacy, uBO filters/privacy/badware, URLhaus | P2 |
| CNAME uncloaking | Chromium's built-in resolver + DoH; check both the original and the canonical name (as Brave does) | P2 |
| Tracking URL parameters | Strip `utm_*`, `fbclid`, `gclid`, `mc_eid`, ... on navigation | P2 |
| Referrer | `strict-origin-when-cross-origin`; cross-site gets the origin only | P2 |
| WebRTC | Hide local IPs (mDNS), no non-proxied UDP | P2 |
| GPC | `Sec-GPC: 1` by default | P2 |
| DNS | DoH with a chosen resolver, ECH | P2 |
| State partitioning | Chromium's built-in; add partitioning for HSTS and caches after Cromite | P5 |
| Anti-fingerprinting | Farbling after Brave (canvas/audio/WebGL noise seeded per site and session); Battery, MIDI, Bluetooth APIs off | P5 |
| Bounce tracking | Chromium's built-in mitigation + lists | P5 |

## 4. Proof: the egress test

`tools/egress-test`, in CI on every release:

1. Start with a new `--user-data-dir`, `--log-net-log=out.json --net-log-capture-mode=Everything`.
2. Scenario A: 10 minutes idle. Expected: **zero hosts**.
3. Scenario B: open a local test page (`127.0.0.1`), the palette, settings, the extension manager.
   Expected: only `127.0.0.1`.
4. Scenario C: enable each opt-in (Widevine, filter updates, LLM endpoint) one at a time. Expected:
   only the declared hosts; the LLM endpoint only after a `?` request, never at startup or idle.
5. A netlog parser lists the hosts and diffs them against `allowlist.yaml`. Any new host fails
   the build.
6. Manual recheck before major versions: Little Snitch or mitmproxy on a separate machine.

Reference point: privacytests.org (the 2026-11-09 run includes Brave 157 and ungoogled 156).

## 5. What we do not copy

- Vivaldi: a daily ping with a unique ID; Google Safe Browsing on by default.
- Opera: an installation ID plus a machine ID, kept up to 3 years; a "fraud check" that sends the
  domains of visited sites; ads in Speed Dial.
- Brave: P3A analytics on by default.
