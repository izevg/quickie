// Quickie's video script, in Quickie's own world of every frame of a page: the main frame and
// each <iframe>, whatever its origin (a YouTube embed on a blog).
//
// Each frame names itself with a random token and reports the state of its selected video,
// preferring playback with sound: `A` playing with sound on, `M` playing muted, `P` paused or ended, `N`
// no video (none played yet, or it went away), then the token. It says `N` once as it starts,
// so the browser knows the frame, and `X` when its document goes away. The browser decides
// which frame's video is the page's. While floated, the selected video keeps its controls.
//
// It hears the media events of the document, which reach the window in their capture phase,
// and of every open shadow root it finds: a media event does not leave its shadow tree. It
// finds shadow roots under every element added to the document or to a shadow root it knows,
// and looks through the whole document again on load and shortly after the user presses a key
// or the pointer, which is how an audible video starts. A closed shadow root is out of reach.
//
// `quickieVideo.float()` makes that video fill the frame's viewport and nothing else show, for
// the mini player, and answers the frame's description (`describe`), or '' when there is no
// video to float. `quickieVideo.hold(description)` does the same with the <iframe> that holds
// the frame described, for a video in a frame: the browser asks each frame in turn, up to the
// main frame. `quickieVideo.dock()` puts the frame back as it was, scrolled where it was.
//
// The element floated stays where it is in the document, so it plays on. Its ancestors, in the
// flat tree through slots and shadow hosts, lose what would trap a fixed element (a transform,
// a filter, containment) while it floats, and one style sheet per tree with marked elements
// does it, so the page's own inline styles, which its player rewrites as it resizes, cannot
// undo it.
//
// `quickieVideo.play(on)` plays or pauses that video.
(() => {
  const token = Math.random().toString(36).slice(2) + Date.now().toString(36);
  const post = (said) => {
    try {
      window.webkit.messageHandlers.quickieVideo.postMessage(said + token);
    } catch (_) {}
  };

  // Shadow roots that have the listeners, and those still attached, to search for a video.
  const heard = new WeakSet();
  const roots = new Set();
  const played = new WeakMap();
  let clock = 0;
  let current = null;
  let last = null;
  let floated = null;

  const trees = () => {
    const all = [document];
    for (const root of roots) {
      if (root.host.isConnected) {
        all.push(root);
      } else {
        roots.delete(root);
      }
    }
    return all;
  };
  const playing = (video) => video?.isConnected && !video.paused && !video.ended;
  const audible = (video) => playing(video) && !video.muted && video.volume > 0;
  const floatedVideo = () => floated?.target instanceof HTMLVideoElement ? floated.target : null;
  const preferred = (video, current) => {
    if (!current) return true;
    const withSound = audible(video);
    if (withSound !== audible(current)) return withSound;
    return (played.get(video) ?? 0) > (played.get(current) ?? 0);
  };
  const findPlaying = (best) => {
    for (const tree of trees()) {
      for (const video of tree.querySelectorAll('video')) {
        if (playing(video) && preferred(video, best)) best = video;
      }
    }
    return best;
  };
  const stateOf = (video) => {
    if (!video?.isConnected) return 'N';
    if (!playing(video)) return 'P';
    return audible(video) ? 'A' : 'M';
  };
  const state = (reselect) => {
    const pinned = floatedVideo();
    if (pinned) return stateOf(pinned);
    if (!current?.isConnected) current = findPlaying(null);
    else if ((reselect || last === 'A') && !audible(current)) current = findPlaying(current);
    return stateOf(current);
  };
  const report = (reselect = false) => {
    const now = state(reselect);
    if (now !== last) {
      last = now;
      post(now);
    }
  };
  const media = (event) => {
    const video = event.target;
    if (!(video instanceof HTMLVideoElement)) return;
    const started = event.type === 'play' || event.type === 'playing';
    if (started) played.set(video, ++clock);
    const unmuted = event.type === 'volumechange' && audible(video) && !audible(current);
    if ((started && (audible(video) || !audible(current))) || unmuted) current = video;
    report();
  };
  const types = ['play', 'playing', 'pause', 'ended', 'emptied', 'volumechange'];
  for (const type of types) addEventListener(type, media, true);

  // Finds the open shadow roots of `node` and of every element under it.
  const scan = (node) => {
    if (node.shadowRoot) hear(node.shadowRoot);
    if (!node.querySelectorAll) return;
    for (const element of node.querySelectorAll('*')) {
      if (element.shadowRoot) hear(element.shadowRoot);
    }
  };
  const added = (records) => {
    const before = roots.size;
    for (const record of records) {
      for (const node of record.addedNodes) {
        if (node.nodeType === Node.ELEMENT_NODE) scan(node);
      }
    }
    const active = floatedVideo() || current;
    if (roots.size !== before || (active && !active.isConnected)) report();
  };
  const hear = (root) => {
    roots.add(root);
    if (heard.has(root)) return;
    heard.add(root);
    for (const type of types) root.addEventListener(type, media, true);
    new MutationObserver(added).observe(root, { childList: true, subtree: true });
    scan(root);
  };
  const rescan = () => {
    const before = roots.size;
    scan(document);
    if (roots.size !== before) report();
  };
  let soon = false;
  const later = () => {
    if (soon) return;
    soon = true;
    setTimeout(() => {
      soon = false;
      rescan();
    }, 250);
  };
  new MutationObserver(added).observe(document, { childList: true, subtree: true });
  addEventListener('load', rescan, true);
  for (const type of ['pointerdown', 'keydown']) addEventListener(type, later, true);

  addEventListener('pagehide', () => {
    last = null;
    post('X');
  }, true);
  addEventListener('pageshow', (event) => {
    if (event.persisted) report();
  }, true);
  report();

  // `:not(#…)` counts as an id, so these rules outweigh a page's own rules on its player.
  const strong = ':not(#quickie-none):not(#quickie-none):not(#quickie-none)';
  const unchained = `transform: none !important; filter: none !important;
  contain: none !important; will-change: auto !important; perspective: none !important;
  backdrop-filter: none !important; clip-path: none !important; mask: none !important;
  z-index: 2147483647 !important;`;
  const sheet = `
[data-quickie-chain]${strong} { ${unchained} }
html[data-quickie-chain]${strong} { overflow: hidden !important; }
[data-quickie-video]${strong} {
  position: fixed !important; inset: 0 !important; width: 100vw !important;
  height: 100vh !important; max-width: none !important; max-height: none !important;
  margin: 0 !important; padding: 0 !important; border: 0 !important;
  object-fit: contain !important; z-index: 2147483647 !important;
  background: #000 !important; transform: none !important; display: block !important;
  visibility: visible !important; opacity: 1 !important;
}`;
  // In a shadow tree, its host's own `:host` rules outweigh the outer tree's when important.
  const hostSheet = `:host([data-quickie-chain])${strong} { ${unchained} }`;

  // The element `element` is laid out in: its slot, its parent, or its shadow root's host.
  const up = (element) => {
    if (element.assignedSlot) return element.assignedSlot;
    if (element.parentElement) return element.parentElement;
    const root = element.parentNode;
    return root && root.host ? root.host : null;
  };
  const describe = () =>
    JSON.stringify({ token, href: location.href, width: innerWidth, height: innerHeight });

  const fill = (target) => {
    const chain = [];
    const scopes = new Set([target.getRootNode()]);
    for (let element = up(target); element; element = up(element)) {
      element.setAttribute('data-quickie-chain', '');
      chain.push(element);
      scopes.add(element.getRootNode());
    }
    target.setAttribute('data-quickie-video', '');
    const sheets = [];
    for (const scope of scopes) {
      const style = document.createElement('style');
      const shadow = scope !== document && scope.host;
      style.textContent = shadow ? sheet + hostSheet : sheet;
      (shadow ? scope : document.documentElement).appendChild(style);
      sheets.push(style);
    }
    floated = { target, chain, sheets, x: scrollX, y: scrollY };
  };
  const floating = (target) => {
    if (!target) return '';
    if (floated) return floated.target === target ? describe() : '';
    fill(target);
    return describe();
  };

  // The <iframe> in this frame that holds the frame described: the one whose script has the
  // token, when this frame can see into it (the same origin), else the best match of its
  // address and of its size with the frame's viewport. A frame of another origin cannot be
  // told apart from a twin (the same address and size) any surer than that.
  const frameOf = (wanted) => {
    let best = null;
    let most = 0;
    for (const tree of trees()) {
      for (const frame of tree.querySelectorAll('iframe')) {
        try {
          const inner = frame.contentWindow && frame.contentWindow.quickieVideo;
          if (inner && inner.token === wanted.token) return frame;
        } catch (_) {}
        let points = 0;
        try {
          const at = new URL(frame.src, location.href);
          const there = new URL(wanted.href);
          if (at.href === there.href) {
            points += 4;
          } else if (at.origin === there.origin && at.pathname === there.pathname) {
            points += 3;
          } else if (at.origin === there.origin) {
            points += 1;
          }
        } catch (_) {}
        if (Math.abs(frame.clientWidth - wanted.width) <= 1 &&
            Math.abs(frame.clientHeight - wanted.height) <= 1) {
          points += 2;
        }
        if (points > most) {
          best = frame;
          most = points;
        }
      }
    }
    // The same origin alone is too weak: a page may hold many frames of one site.
    return most >= 2 ? best : null;
  };

  window.quickieVideo = {
    token,
    float() {
      return floating(current && current.isConnected ? current : null);
    },
    hold(description) {
      let wanted;
      try {
        wanted = JSON.parse(description);
      } catch (_) {
        return '';
      }
      return floating(frameOf(wanted));
    },
    dock() {
      if (!floated) return;
      const { target, chain, sheets, x, y } = floated;
      floated = null;
      for (const style of sheets) style.remove();
      target.removeAttribute('data-quickie-video');
      for (const element of chain) element.removeAttribute('data-quickie-chain');
      report(target instanceof HTMLVideoElement);
      // The view is back at its size a frame or two later; scroll once it laid out again.
      requestAnimationFrame(() => requestAnimationFrame(() => scrollTo(x, y)));
    },
    play(on) {
      const video = floatedVideo() || current;
      if (!video?.isConnected) return;
      if (on) {
        video.play().catch(() => {});
      } else {
        video.pause();
      }
    },
  };
})();
