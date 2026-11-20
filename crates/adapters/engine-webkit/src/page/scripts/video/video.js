// Quickie's video script, in Quickie's own world of a page's main frame.
//
// It reports what the video the page played last does, on every change: `A` playing with its
// sound on, `M` playing muted, `P` paused or ended, `N` no video (none played yet, it went
// away, or the page is leaving). It hears the media events of the document, which reach the
// window in their capture phase.
//
// `quickieVideo.float()` makes that video fill the viewport and nothing else show, for the
// mini player; `quickieVideo.dock()` puts the page back as it was, scrolled where it was.
// The video element stays where it is in the document, so it plays on. Its ancestors lose
// what would trap a fixed element (a transform, a filter, containment) while it floats, and
// one style sheet with marked elements does it, so the page's own inline styles, which its
// player rewrites as it resizes, cannot undo it.
//
// `quickieVideo.play(on)` plays or pauses that video.
(() => {
  const post = (report) => {
    try {
      window.webkit.messageHandlers.quickieVideo.postMessage(report);
    } catch (_) {}
  };
  let current = null;
  let last = 'N';
  const state = () => {
    if (current && !current.isConnected) current = null;
    if (!current) {
      current = Array.from(document.querySelectorAll('video'))
        .find((video) => !video.paused && !video.ended) || null;
    }
    if (!current) return 'N';
    if (current.paused || current.ended) return 'P';
    return current.muted || current.volume === 0 ? 'M' : 'A';
  };
  const report = () => {
    const now = state();
    if (now !== last) {
      last = now;
      post(now);
    }
  };
  const heard = (event) => {
    const video = event.target;
    if (!(video instanceof HTMLVideoElement)) return;
    if (event.type === 'play' || event.type === 'playing') current = video;
    report();
  };
  for (const type of ['play', 'playing', 'pause', 'ended', 'emptied', 'volumechange']) {
    addEventListener(type, heard, true);
  }
  addEventListener('pagehide', () => {
    if (last !== 'N') {
      last = 'N';
      post('N');
    }
  }, true);
  addEventListener('pageshow', (event) => {
    if (event.persisted) report();
  }, true);

  // `:not(#…)` counts as an id, so these rules outweigh a page's own rules on its player.
  const strong = ':not(#quickie-none):not(#quickie-none):not(#quickie-none)';
  const sheet = `
[data-quickie-chain]${strong} {
  transform: none !important; filter: none !important; contain: none !important;
  will-change: auto !important; perspective: none !important;
  backdrop-filter: none !important; clip-path: none !important; mask: none !important;
  z-index: 2147483647 !important;
}
html[data-quickie-chain]${strong} { overflow: hidden !important; }
video[data-quickie-video]${strong} {
  position: fixed !important; inset: 0 !important; width: 100vw !important;
  height: 100vh !important; max-width: none !important; max-height: none !important;
  margin: 0 !important; padding: 0 !important; object-fit: contain !important;
  z-index: 2147483647 !important; background: #000 !important; transform: none !important;
  visibility: visible !important; opacity: 1 !important;
}`;
  let floated = null;
  window.quickieVideo = {
    float() {
      const video = current && current.isConnected ? current : null;
      if (!video || floated) return;
      const chain = [];
      for (let element = video.parentElement; element; element = element.parentElement) {
        element.setAttribute('data-quickie-chain', '');
        chain.push(element);
      }
      video.setAttribute('data-quickie-video', '');
      const style = document.createElement('style');
      style.textContent = sheet;
      document.documentElement.appendChild(style);
      floated = { video, chain, style, x: scrollX, y: scrollY };
    },
    dock() {
      if (!floated) return;
      const { video, chain, style, x, y } = floated;
      floated = null;
      style.remove();
      video.removeAttribute('data-quickie-video');
      for (const element of chain) element.removeAttribute('data-quickie-chain');
      // The view is back at its size a frame or two later; scroll once it laid out again.
      requestAnimationFrame(() => requestAnimationFrame(() => scrollTo(x, y)));
    },
    play(on) {
      const video = current;
      if (!video) return;
      if (on) {
        video.play().catch(() => {});
      } else {
        video.pause();
      }
    },
  };
})();
