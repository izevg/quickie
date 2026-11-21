// EXPERIMENT (never merged): frame timing as the page sees it, for the video stutter hunt.
// Every 250 ms it posts one JSON line: the requestAnimationFrame gaps, every video frame the
// compositor presented (requestVideoFrameCallback metadata), playback quality, and the page's
// visibility and focus. Times are Unix milliseconds (performance.timeOrigin + ...).
(() => {
  if (window.top !== window) return;
  const origin = performance.timeOrigin;
  const post = (record) => {
    try {
      window.webkit.messageHandlers.quickieTrace.postMessage(JSON.stringify(record));
    } catch (_) {}
  };
  const r2 = (x) => Math.round(x * 100) / 100;

  let lastRaf = 0;
  let raf = [];
  const onRaf = (t) => {
    const now = origin + t;
    if (lastRaf) raf.push(r2(now - lastRaf));
    lastRaf = now;
    requestAnimationFrame(onRaf);
  };
  requestAnimationFrame(onRaf);

  let frames = [];
  const watched = new WeakSet();
  const watch = (video) => {
    if (watched.has(video) || !video.requestVideoFrameCallback) return;
    watched.add(video);
    const onFrame = (now, m) => {
      frames.push([
        r2(origin + now),
        r2(origin + m.expectedDisplayTime),
        m.presentedFrames,
        Math.round(m.mediaTime * 10000) / 10000,
        m.presentationTime ? r2(origin + m.presentationTime) : null,
        m.processingDuration === undefined ? null : r2(m.processingDuration * 1000),
      ]);
      video.requestVideoFrameCallback(onFrame);
    };
    video.requestVideoFrameCallback(onFrame);
  };

  setInterval(() => {
    const videos = Array.from(document.querySelectorAll('video'));
    videos.forEach(watch);
    const playing = videos.find((v) => !v.paused && !v.ended);
    const quality = playing && playing.getVideoPlaybackQuality
      ? playing.getVideoPlaybackQuality() : null;
    post({
      k: 'page',
      t: r2(origin + performance.now()),
      vis: document.visibilityState,
      focus: document.hasFocus(),
      raf,
      vf: frames,
      q: quality && [quality.droppedVideoFrames, quality.totalVideoFrames],
      v: playing ? [playing.videoWidth, playing.videoHeight, r2(playing.currentTime),
        playing.buffered.length ? r2(playing.buffered.end(playing.buffered.length - 1)) : null]
        : null,
      dpr: window.devicePixelRatio,
      scr: [screen.width, screen.height],
    });
    raf = [];
    frames = [];
  }, 250);
})();
