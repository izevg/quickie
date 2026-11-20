// Quickie's cosmetic filtering, in Quickie's shields content world: the page's scripts cannot
// see these globals, call the message handler, or replace the functions used here.
//
// At document start in every frame: hides what the lists hide on this host (one constructed
// stylesheet, a rule per selector, so one invalid selector costs only its own rule), runs the
// procedural filters as the page changes, and reports the classes and ids the page uses, in
// batches, so that Quickie can answer with the generic rules that hide them.
(() => {
  'use strict';
  const data = /*QUICKIE_SHIELDS_DATA*/ null;
  if (!data) return;
  const host = location.hostname;
  const entry = (data.hosts && data.hosts[host]) || {};
  if (entry.off) return;

  const removed = new Set(entry.remove || []);
  const generic = entry.generichide ? [] : (data.generic || []).filter((s) => !removed.has(s));
  const hide = (selectors) => selectors.map((s) => `${s}{display:none!important}`).join('\n');
  const sheet = new CSSStyleSheet();
  try {
    sheet.replaceSync(`${hide(generic)}\n${(entry.css || []).join('\n')}`);
  } catch (_) {}
  const adopt = () => {
    try {
      if (!document.adoptedStyleSheets.includes(sheet)) {
        document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
      }
    } catch (_) {}
  };
  adopt();

  const added = new Set();
  const addSelectors = (selectors) => {
    for (const selector of selectors) {
      if (added.has(selector)) continue;
      added.add(selector);
      try {
        sheet.insertRule(`${selector}{display:none!important}`, sheet.cssRules.length);
      } catch (_) {}
    }
    adopt();
  };

  // Generic class and id rules: report each new class or id once, in batches.
  const handler = window.webkit && window.webkit.messageHandlers &&
    window.webkit.messageHandlers.quickieShields;
  const report = !entry.generichide && handler;
  const seenClasses = new Set();
  const seenIds = new Set();
  let classes = [];
  let ids = [];
  let flushing = 0;
  const flush = () => {
    flushing = 0;
    if (classes.length === 0 && ids.length === 0) return;
    const message = JSON.stringify({ host, classes, ids });
    classes = [];
    ids = [];
    handler.postMessage(message).then((reply) => {
      try {
        addSelectors(JSON.parse(reply));
      } catch (_) {}
    }, () => {});
  };
  const note = (element) => {
    if (!report || element.nodeType !== 1) return;
    const id = element.id;
    if (id && !seenIds.has(id)) {
      seenIds.add(id);
      ids.push(id);
    }
    const list = element.classList;
    if (list) {
      for (const name of list) {
        if (!seenClasses.has(name)) {
          seenClasses.add(name);
          classes.push(name);
        }
      }
    }
    if (!flushing && (classes.length || ids.length)) flushing = setTimeout(flush, 30);
  };
  const noteTree = (root) => {
    note(root);
    if (root.querySelectorAll) {
      for (const element of root.querySelectorAll('[id],[class]')) note(element);
    }
  };

  // Procedural filters (`:has-text()`, `:upward()`, `:xpath()`, ... and actions such as
  // `:remove()` or `:style()`), as adblock-rust describes them in JSON.
  const pattern = (text) => {
    const re = /^\/(.+)\/([imsu]*)$/.exec(text);
    if (re) {
      const compiled = new RegExp(re[1], re[2]);
      return (subject) => compiled.test(subject);
    }
    return (subject) => subject.includes(text);
  };
  const unquote = (text) => text.trim().replace(/^"(.*)"$/, '$1');
  const operators = {
    'css-selector': (nodes, arg, first) => {
      if (first) return [...document.querySelectorAll(arg)];
      const found = [];
      for (const node of nodes) found.push(...node.querySelectorAll(`:scope ${arg}`));
      return found;
    },
    'has-text': (nodes, arg) => {
      const test = pattern(arg);
      return nodes.filter((node) => test(node.textContent || ''));
    },
    'min-text-length': (nodes, arg) =>
      nodes.filter((node) => (node.textContent || '').length >= Number(arg)),
    'matches-attr': (nodes, arg) => {
      const eq = arg.indexOf('=');
      const name = pattern(unquote(eq === -1 ? arg : arg.slice(0, eq)));
      const value = eq === -1 ? () => true : pattern(unquote(arg.slice(eq + 1)));
      return nodes.filter((node) =>
        [...node.attributes].some((attr) => name(attr.name) && value(attr.value)));
    },
    'matches-css': (nodes, arg, first, pseudo) => {
      const colon = arg.indexOf(':');
      if (colon === -1) return [];
      const prop = arg.slice(0, colon).trim();
      const test = pattern(arg.slice(colon + 1).trim());
      return nodes.filter((node) =>
        test(getComputedStyle(node, pseudo || null).getPropertyValue(prop)));
    },
    'matches-css-before': (nodes, arg) => operators['matches-css'](nodes, arg, false, '::before'),
    'matches-css-after': (nodes, arg) => operators['matches-css'](nodes, arg, false, '::after'),
    'matches-path': (nodes, arg) =>
      pattern(arg)(location.pathname + location.search) ? nodes : [],
    upward: (nodes, arg) => {
      const steps = Number(arg);
      return nodes.map((node) => {
        if (Number.isInteger(steps) && steps > 0) {
          let up = node;
          for (let step = 0; up && step < steps; step += 1) up = up.parentElement;
          return up;
        }
        return node.parentElement ? node.parentElement.closest(arg) : null;
      }).filter(Boolean);
    },
    xpath: (nodes, arg, first) => {
      const found = [];
      for (const node of first ? [document] : nodes) {
        const result = document.evaluate(arg, node, null,
          XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
        for (let at = 0; at < result.snapshotLength; at += 1) {
          const item = result.snapshotItem(at);
          if (item && item.nodeType === 1) found.push(item);
        }
      }
      return found;
    },
  };
  const actions = {
    undefined: (node) => node.style.setProperty('display', 'none', 'important'),
    remove: (node) => node.remove(),
    style: (node, arg) => { node.style.cssText += `;${arg}`; },
    'remove-attr': (node, arg) => node.removeAttribute(arg),
    'remove-class': (node, arg) => node.classList.remove(arg),
  };
  const procedural = (entry.procedural || []).map((text) => {
    try {
      return JSON.parse(text);
    } catch (_) {
      return null;
    }
  }).filter((filter) => filter && Array.isArray(filter.selector) && filter.selector.length);
  const runProcedural = () => {
    for (const filter of procedural) {
      try {
        let nodes = [];
        filter.selector.forEach((step, at) => {
          const operator = operators[step.type];
          nodes = operator ? operator(nodes, String(step.arg), at === 0) : [];
        });
        const act = actions[filter.action ? filter.action.type : undefined];
        if (!act) continue;
        for (const node of new Set(nodes)) act(node, filter.action && filter.action.arg);
      } catch (_) {}
    }
  };

  let proceduralQueued = false;
  const queueProcedural = () => {
    if (procedural.length === 0 || proceduralQueued) return;
    proceduralQueued = true;
    setTimeout(() => {
      proceduralQueued = false;
      runProcedural();
    }, 50);
  };

  // Watched from document start, so elements are seen as the parser adds them, before they
  // are painted.
  if (document.documentElement) noteTree(document.documentElement);
  new MutationObserver((records) => {
    for (const record of records) {
      if (record.type === 'attributes') note(record.target);
      else for (const node of record.addedNodes) if (node.nodeType === 1) noteTree(node);
    }
    queueProcedural();
  }).observe(document, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['class', 'id'],
  });
  const ready = () => {
    adopt();
    runProcedural();
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', ready, { once: true });
  } else {
    ready();
  }
})();
