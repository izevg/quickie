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

### Filter lists

- Lists come only from the allowed sources (EasyList, EasyPrivacy, uBlock filters, uBlock quick
  fixes, Peter Lowe's), chosen by id, always over https. None ships with Quickie; their licenses
  are not ours.
- A fresh profile has every list off, so Shields opens no connection on its own.
- Enabling a list is the request to fetch it, once. After that a list is fetched again only when
  the user asks ("Refresh Filter Lists"), or, if the user turned on "keep lists fresh", once it is
  four days old.
- A scheduled refresh counts as a connection made on the browser's own initiative: the user did
  not ask for it at that moment. That is why "keep lists fresh" is off by default, and why the
  egress test's scenario C lists the filter list hosts as an opt-in.
- Fetching goes through the engine's network stack (`HttpPort`): Chromium's network service in
  the product, `NSURLSession` in the development build.

### Cosmetic filters and scriptlets (development engine)

Network rules cannot block ads a site serves from its own domain, as video sites do. The lists
answer those with cosmetic filters (hide this element, run this procedure) and scriptlets
(`##+js(json-prune, ...)`: small scripts that change the page's own objects before its scripts
run). On the WebKit engine, `adblock` looks up what each host gets, off the main thread, and
Quickie injects it at document start in every frame:

- **Scriptlets run in the page's world.** They must patch the page's own `JSON.parse`, `fetch`,
  and globals, as in uBlock Origin. They are wrapped in a closure: nothing of Quickie's is
  reachable from the page, and they talk to nothing.
- **Hiding runs in Quickie's own content world** (`quickie-shields`): a constructed stylesheet,
  the procedural filters, and a batched report of the classes and ids the page uses, answered
  with the generic rules for them. Only that world has the message handler, and it answers with
  selectors, nothing else. The names the page wrote are bounded before they are read.
- **Trusted scriptlets** (those that rewrite network answers or set any value) run only from
  uBlock Origin's own lists, as uBlock Origin does; EasyList and the others get the ordinary ones.
- A site with shields off gets no cosmetic filter and no scriptlet.
- Nothing extra is fetched: the scriptlet library ships in the binary.

**Licensing of the scriptlet library (decided).** `adblock` needs a library of scriptlet code to
run `+js(...)` rules. The candidates and why they were not taken:

| Source | License | Verdict |
|---|---|---|
| uBlock Origin's resources (`src/js/resources/`) | GPL-3.0 | Cannot be vendored or shipped (AGENTS.md: no GPL family) |
| AdGuard Scriptlets | GPL-3.0 | Same |
| Brave's `adblock-resources` | MPL-2.0 for Brave's own few, but its built `resources.json` bundles uBlock Origin's GPL scriptlets | Same, as built |
| Fetching uBlock Origin's resources at runtime, as data, like the lists | Not distributed by us, so not a license problem | Rejected: it would inject code nobody here reviewed into every page with the page's privileges, add a connection, and need a JavaScript build step (the resources are ES modules with imports that `adblock` 0.13 does not assemble) |
| Quickie's own library | Apache-2.0 | **Chosen** |

Quickie's library (`crates/adapters/engine-webkit/src/shields/cosmetic/js/scriptlets.js`) is
written for Quickie from the documented behavior of each scriptlet; it follows uBlock Origin's
names, aliases, and arguments, which are the filter syntax the lists are written in, and carries
none of its code. It covers the scriptlets video and ad rules use most: `set-constant`,
`json-prune` and its `fetch`/`XMLHttpRequest` response variants, `trusted-replace-fetch-response`
and `trusted-replace-xhr-response`, `abort-on-property-read/write`, `abort-current-script`, the
timer and `addEventListener` defusers, `no-fetch-if`, `no-xhr-if`, `noeval-if`,
`no-window-open-if`, `remove-attr`, `remove-class`, the cookie and storage setters, and the
trusted object and text rewriters. Paths take the documented wildcards (`[]`, `*`) and removal
tokens (`[-]` drops each array element and `{-}` each property in which the rest of the path
exists, as the Shorts feed's `entries.[-]...adClientParams.isAd` rules need). A rule that names a
scriptlet the library lacks is skipped.

**Blocked counts.** WebKit reports what a content rule list did only through a private navigation
delegate callback, `_webView:contentRuleListWithIdentifier:performedAction:forURL:` (the one
Safari's and DuckDuckGo's blockers count with). Quickie implements it on its navigation delegate;
WebKit calls it only because the delegate responds to it, and the private action object is asked
for `blockedLoad` only when it responds to that selector. Without it the counts stay at zero.
Counts live in memory per page and are never stored.

### Browser updates

- **Check for Updates** queries the public Quickie repository on GitHub. Automatic updates
  are off in a fresh profile. With automatic updates enabled, the browser checks at most
  once a day while it is running.
- The updater uses the engine's networking libraries in a separate session without cookies or
  credentials. It downloads only the matching package from the release and checks the reported
  size and SHA-256 before installation. Installed release notes are read from the package.
- After system authorization for a DEB or RPM update, the updater checks the GitHub release
  again before invoking the package manager. The system package manager may contact configured
  distribution repositories to resolve dependencies.

### Web extensions

- A fresh profile has no extension, and installing one is always the user's request: a Chrome
  Web Store page pasted into the palette or open in the tab ("Install Extension from URL…"), or
  a `.crx`, `.zip`, or folder chosen on disk. Nothing is suggested, preinstalled, or fetched
  in the background.
- Installing from the store asks Google's update service once (`clients2.google.com`), then
  downloads the package from the host it names, accepted only on Google's update hosts over
  https. The package is kept only if its CRX3 signatures hold and prove the extension's id, and
  its SHA-256 matches what the store announced.
- Updates come only when the user asks ("Check Extension Updates"), or, if the user turned on
  "Update automatically", once a day. That is a connection on the browser's own initiative, so
  it is off by default, like "keep lists fresh".
- Every request goes through the engine's network stack (`HttpPort`), with no cookies or
  credentials. What an extension itself fetches is page traffic of the main profile.
- An extension runs only after the user approved what it asks for in a sheet, and gets only
  that; a later request for more asks again. It never runs in the agent's profile and gets no
  access to throwaway jars' tabs and data. On the system `WebKit` throwaway pages have their
  own user content controller, with the same content rule lists and shields scripts but none
  of the extensions' content scripts, so those never reach a throwaway page.

### Downloads in a throwaway jar

- The file is saved to `~/Downloads` like any other: the user asked for it.
- It is listed in the downloads popover while Quickie runs, so it can be watched, cancelled, and
  opened, but it is never written to `shell.sqlite`. After a restart the library keeps no trace
  of where the file came from, just as the jar's visits never reach history.
- A cancelled or failed download's partial file is deleted. Quickie never overwrites a file in
  `~/Downloads`: a taken name gets the next number (`report 2.pdf`).

### Page zoom and saved pages in a throwaway jar

- A throwaway tab starts at 100 %, whatever the main jar chose for the site. What it zooms is
  kept for the jar while Quickie runs and dropped with it; it is never written to
  `shell.sqlite`.
- A page saved as a PDF goes to `~/Downloads` like a download (the user asked for it) and, from
  a throwaway jar, is listed only while Quickie runs.

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
