// The Fullscreen API in the tab's own window, as Chrome does it, instead of WebKit's separate
// full-screen window. Installed at document start in the page's own world of every frame, so
// it replaces `requestFullscreen` and friends before the page's scripts run; everything it
// calls later was taken from the prototypes before the page could change them.
//
// An element that asks to go full screen moves into the top layer (a manual popover) and fills
// the page view; the page under it stops scrolling. A frame's element fills its frame, and the
// frame's parent makes that frame fill its own view, up to the main frame. The main frame tells
// Quickie's world with a `quickie-fullscreen-enter` or `quickie-fullscreen-exit` event on the
// document; that world checks the user's activation and tells the browser, which takes the
// window full screen. The browser takes the page out with a `quickie-fullscreen-leave` event.
// Leaving puts the element's style and the scroll position back as they were.
//
// Not replaced: the `:fullscreen` pseudo-class, and the fullscreen button of WebKit's own media
// controls, which still uses WebKit's full-screen window.
(() => {
  const uncurry = (f) => Function.prototype.call.bind(f);
  const getter = (proto, name) => {
    const found = proto && Object.getOwnPropertyDescriptor(proto, name);
    return found && found.get ? uncurry(found.get) : null;
  };
  const D = Document.prototype;
  const E = Element.prototype;
  const Video = HTMLVideoElement;
  const V = Video.prototype;
  const nativeEnabled = getter(D, 'fullscreenEnabled') || getter(D, 'webkitFullscreenEnabled');
  const activation = getter(Navigator.prototype, 'userActivation');
  const isActive = window.UserActivation ? getter(UserActivation.prototype, 'isActive') : null;
  const getAttribute = uncurry(E.getAttribute);
  const setAttribute = uncurry(E.setAttribute);
  const hasAttribute = uncurry(E.hasAttribute);
  const removeAttribute = uncurry(E.removeAttribute);
  const isConnected = getter(Node.prototype, 'isConnected');
  const getRootNode = uncurry(Node.prototype.getRootNode);
  const querySelectorAll = uncurry(D.querySelectorAll);
  const contentWindow = getter(HTMLIFrameElement.prototype, 'contentWindow');
  const showPopover = HTMLElement.prototype.showPopover
    ? uncurry(HTMLElement.prototype.showPopover) : null;
  const hidePopover = HTMLElement.prototype.hidePopover
    ? uncurry(HTMLElement.prototype.hidePopover) : null;
  const styleOf = getter(HTMLElement.prototype, 'style');
  const setProperty = uncurry(CSSStyleDeclaration.prototype.setProperty);
  const getPropertyValue = uncurry(CSSStyleDeclaration.prototype.getPropertyValue);
  const getPropertyPriority = uncurry(CSSStyleDeclaration.prototype.getPropertyPriority);
  const computed = window.getComputedStyle;
  const dispatch = uncurry(EventTarget.prototype.dispatchEvent);
  const listen = uncurry(EventTarget.prototype.addEventListener);
  // `postMessage` is each window's own (Window is the global), so it is called on the target.
  const post = (target, message) => target.postMessage(message, '*');
  const Observer = window.MutationObserver;
  const observe = uncurry(Observer.prototype.observe);
  const disconnect = uncurry(Observer.prototype.disconnect);
  const later = window.setTimeout;
  const scrollTo = window.scrollTo;
  const now = Date.now;
  const Ev = window.Event;
  const Err = window.TypeError;
  const resolved = Promise.resolve.bind(Promise);
  const rejected = Promise.reject.bind(Promise);
  const catchPromise = uncurry(Promise.prototype.catch);
  const top = window.top === window;
  const doc = document;
  const MARK = 'quickieFullscreen';

  // What the element filling the view gets, over anything the page set.
  const FILL = [
    ['position', 'fixed'], ['inset', '0px'], ['top', '0px'], ['left', '0px'],
    ['right', '0px'], ['bottom', '0px'], ['width', '100%'], ['height', '100%'],
    ['min-width', '0px'], ['min-height', '0px'], ['max-width', 'none'], ['max-height', 'none'],
    ['margin', '0px'], ['box-sizing', 'border-box'], ['transform', 'none'],
    ['translate', 'none'], ['scale', 'none'], ['rotate', 'none'], ['z-index', '2147483647'],
    ['object-fit', 'contain'], ['visibility', 'visible'], ['opacity', '1'],
  ];
  // What the top layer's popover style changes, kept at what the page's own style computed.
  const KEPT = [
    'border-top-style', 'border-right-style', 'border-bottom-style', 'border-left-style',
    'border-top-width', 'border-right-width', 'border-bottom-width', 'border-left-width',
    'border-top-color', 'border-right-color', 'border-bottom-color', 'border-left-color',
    'padding-top', 'padding-right', 'padding-bottom', 'padding-left', 'color',
    'overflow-x', 'overflow-y',
  ];

  // The element of this document that fills the view: its own, or the frame that holds the
  // one that does.
  let current = null;
  let saved = null;
  let watcher = null;

  const allowed = () => {
    if (nativeEnabled && !nativeEnabled(doc)) return false;
    const user = activation && activation(navigator);
    return !(user && isActive && !isActive(user));
  };

  const apply = (element) => {
    const style = styleOf(element);
    for (const [name, value] of saved.fill) setProperty(style, name, value, 'important');
  };

  const intact = (element) => {
    const style = styleOf(element);
    return saved.fill.every(([name, value]) =>
      getPropertyValue(style, name) === value && getPropertyPriority(style, name) === 'important');
  };

  const fill = (element) => {
    const root = doc.documentElement;
    if (element === root) {
      // The whole document: it fills the view already.
      saved = { element, fill: [], style: null, rootStyle: null, popover: false, whole: true };
      return;
    }
    const before = computed(element);
    const fillWith = FILL.slice();
    for (const name of KEPT) fillWith.push([name, before.getPropertyValue(name)]);
    const background = before.getPropertyValue('background-color');
    fillWith.push(['background-color',
      background === 'rgba(0, 0, 0, 0)' || background === 'transparent' ? 'black' : background]);
    if (element.localName === 'iframe') {
      for (const side of ['top', 'right', 'bottom', 'left']) {
        fillWith.push([`border-${side}-style`, 'none'], [`padding-${side}`, '0px']);
      }
    }
    saved = {
      element,
      fill: fillWith,
      style: hasAttribute(element, 'style') ? getAttribute(element, 'style') : null,
      rootStyle: root && hasAttribute(root, 'style') ? getAttribute(root, 'style') : null,
      popover: false,
      x: window.scrollX,
      y: window.scrollY,
    };
    apply(element);
    if (root) setProperty(styleOf(root), 'overflow', 'hidden', 'important');
    if (showPopover && !hasAttribute(element, 'popover')) {
      try {
        setAttribute(element, 'popover', 'manual');
        showPopover(element);
        saved.popover = true;
      } catch (_) {
        removeAttribute(element, 'popover');
      }
    }
    watcher = new Observer(() => {
      if (current === element && !intact(element)) apply(element);
    });
    observe(watcher, element, { attributes: true, attributeFilter: ['style'] });
  };

  // Where the page goes back to while the window shrinks back, over a moment.
  let restore = null;
  listen(window, 'resize', () => {
    if (restore && now() < restore.until) scrollTo.call(window, restore.x, restore.y);
  }, true);

  const unfill = () => {
    if (!saved) return;
    if (saved.whole) {
      saved = null;
      return;
    }
    const { element, style, rootStyle, popover, x, y } = saved;
    const root = doc.documentElement;
    if (watcher) disconnect(watcher);
    watcher = null;
    if (popover) {
      try { hidePopover(element); } catch (_) {}
      removeAttribute(element, 'popover');
    }
    if (style === null) removeAttribute(element, 'style');
    else setAttribute(element, 'style', style);
    if (root) {
      if (rootStyle === null) removeAttribute(root, 'style');
      else setAttribute(root, 'style', rootStyle);
    }
    saved = null;
    restore = { x, y, until: now() + 1500 };
    const back = () => scrollTo.call(window, x, y);
    back();
    later(back, 0);
  };

  const fire = (element) => {
    for (const name of ['fullscreenchange', 'webkitfullscreenchange']) {
      const target = element && isConnected(element) ? element : doc;
      dispatch(target, new Ev(name, { bubbles: true, composed: true }));
    }
    if (element instanceof Video) dispatch(element, new Ev('webkitpresentationmodechanged'));
  };

  const tell = (state) => {
    if (top) {
      dispatch(doc, new Ev(`quickie-fullscreen-${state}`));
    } else {
      try { post(window.parent, { [MARK]: state }); } catch (_) {}
    }
  };

  // Makes `element` this document's full-screen element: `own` when the page asked for it,
  // not when it is the frame of a child document that did.
  const enter = (element) => {
    const previous = current;
    if (previous === element) return;
    if (previous) {
      unfill();
      passDown(previous);
    }
    current = element;
    fill(element);
    tell('enter');
    later(() => {
      if (previous) fire(previous);
      fire(element);
    }, 0);
  };

  // Tells the document in `element`, a frame that was full screen, that it is not any more.
  const passDown = (element) => {
    if (element.localName !== 'iframe') return;
    const child = contentWindow(element);
    try { if (child) post(child, { [MARK]: 'leave' }); } catch (_) {}
  };

  // Leaves full screen in this document. `from` is where the leaving came from, so it is not
  // passed back there: `up` from the frame that was full screen, `down` from the parent, and
  // `here` from this document (the page or the browser), passed both ways.
  const leave = (from) => {
    const element = current;
    if (!element) return;
    current = null;
    unfill();
    if (from !== 'down') tell('exit');
    if (from !== 'up') passDown(element);
    later(() => fire(element), 0);
  };

  function requestFullscreen() {
    const element = this;
    if (!(element instanceof Element) || !isConnected(element) || !allowed()) {
      later(() => dispatch(element instanceof Element ? element : doc,
        new Ev('fullscreenerror', { bubbles: true, composed: true })), 0);
      return rejected(new Err('Fullscreen request denied'));
    }
    enter(element);
    return resolved();
  }

  function exitFullscreen() {
    if (!current) return rejected(new Err('Not in fullscreen'));
    leave('here');
    return resolved();
  }

  // The element the page sees: the full-screen element, or the shadow host that holds it.
  const visible = () => {
    let node = current;
    while (node && getRootNode(node) !== doc) node = getRootNode(node).host;
    return node || null;
  };

  const define = (proto, name, value) => {
    try {
      Object.defineProperty(proto, name, { value, configurable: true, writable: true });
    } catch (_) {}
  };
  const read = (proto, name, get) => {
    try {
      Object.defineProperty(proto, name, { get, configurable: true, enumerable: true });
    } catch (_) {}
  };

  for (const name of ['requestFullscreen', 'webkitRequestFullscreen', 'webkitRequestFullScreen']) {
    define(E, name, requestFullscreen);
  }
  for (const name of ['exitFullscreen', 'webkitExitFullscreen', 'webkitCancelFullScreen']) {
    define(D, name, exitFullscreen);
  }
  for (const name of ['fullscreenElement', 'webkitFullscreenElement',
    'webkitCurrentFullScreenElement']) {
    read(D, name, visible);
  }
  for (const name of ['fullscreen', 'webkitIsFullScreen']) read(D, name, () => current !== null);
  function requestVideoFullscreen(video) {
    // The legacy video APIs return void, including when a gesture is missing.
    catchPromise(requestFullscreen.call(video), () => {});
  }

  for (const name of ['webkitEnterFullscreen', 'webkitEnterFullScreen']) {
    define(V, name, function () { requestVideoFullscreen(this); });
  }
  for (const name of ['webkitExitFullscreen', 'webkitExitFullScreen']) {
    define(V, name, function () { if (current === this) leave('here'); });
  }
  read(V, 'webkitDisplayingFullscreen', function () { return current === this; });

  const nativePresentationMode = getter(V, 'webkitPresentationMode');
  if (nativePresentationMode && V.webkitSetPresentationMode) {
    const setNativePresentationMode = uncurry(V.webkitSetPresentationMode);
    read(V, 'webkitPresentationMode', function () {
      return current === this ? 'fullscreen' : nativePresentationMode(this);
    });
    define(V, 'webkitSetPresentationMode', function (mode) {
      if (nativePresentationMode(this) === 'inline') {
        if (mode === 'fullscreen') { requestVideoFullscreen(this); return; }
        if (mode === 'inline' && current === this) { leave('here'); return; }
      }
      setNativePresentationMode(this, mode);
    });
    // WebKit owns picture-in-picture. Leave our fullscreen once the native handoff succeeds.
    listen(doc, 'webkitpresentationmodechanged', (event) => {
      if (current === event.target && current instanceof Video && nativePresentationMode(current) !== 'inline')
        leave('here');
    }, true);
  }

  // A child frame entered or left, or the parent left.
  listen(window, 'message', (event) => {
    const data = event.data;
    if (!data || typeof data !== 'object' || !(MARK in data)) return;
    event.stopImmediatePropagation();
    const state = data[MARK];
    if (state === 'leave') {
      if (event.source === window.parent) leave('down');
      return;
    }
    const frame = Array.prototype.find.call(querySelectorAll(doc, 'iframe'),
      (candidate) => contentWindow(candidate) === event.source);
    if (!frame) return;
    if (state === 'enter') enter(frame);
    else if (state === 'exit' && current === frame) leave('up');
  }, true);

  // The browser takes the page out: `⎋`, the window leaving full screen, another tab.
  listen(doc, 'quickie-fullscreen-leave', () => { leave('here'); }, true);
})();
