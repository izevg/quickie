(() => {
  const textless = new Set([
    'button', 'checkbox', 'color', 'file', 'hidden', 'image', 'radio', 'range', 'reset', 'submit',
  ]);
  const frame = Math.random().toString(36).slice(2) + Date.now().toString(36);
  let last;

  function editable() {
    let element = document.activeElement;
    while (element?.shadowRoot?.activeElement) {
      element = element.shadowRoot.activeElement;
    }
    return !!element && (element.isContentEditable || element.tagName === 'TEXTAREA' ||
      element.tagName === 'SELECT' ||
      (element.tagName === 'INPUT' && !textless.has(element.type)));
  }

  function send(focused, reset = false) {
    last = focused;
    window.webkit.messageHandlers.quickieKeyboardFocus.postMessage(
      (reset ? 'R' : '-') + (focused ? '1' : '0') + frame
    );
  }

  function update() {
    const focused = editable();
    if (focused !== last) send(focused);
  }

  addEventListener('focusin', update, true);
  addEventListener('focusout', () => setTimeout(update, 0), true);
  addEventListener('pageshow', () => send(editable()), true);
  addEventListener('pagehide', () => send(false), true);
  send(editable(), window === window.top);
})();
