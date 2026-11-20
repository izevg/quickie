// Quickie's scriptlet library. Written for Quickie under its own license (Apache-2.0); the
// names and arguments follow uBlock Origin's filter syntax so that `##+js(...)` rules in the
// lists the user enabled work, but none of uBlock Origin's or AdGuard's code is in here.
//
// The page-world shields script evaluates this expression once per frame, before any of the
// page's own scripts, and only on a site that has a scriptlet rule. It returns one function,
// `run(name, args)`, kept in the shields script's closure: nothing here is reachable from the
// page, and nothing here reaches Quickie (no messages, no privileged calls).
(() => {
  'use strict';
  const win = window;
  const { defineProperty, getOwnPropertyDescriptor, keys } = Object;
  const hasOwn = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
  const apply = Reflect.apply;
  const jsonParse = JSON.parse;
  const jsonStringify = JSON.stringify;
  const NativeRegExp = RegExp;
  const NativeResponse = win.Response;
  const NativeProxy = Proxy;
  const fnText = (fn) => {
    try {
      return typeof fn === 'function' ? Function.prototype.toString.call(fn) : String(fn);
    } catch (_) {
      return '';
    }
  };

  // Patterns as filter lists write them: empty or `*` for anything, `/re/flags` for a regular
  // expression, otherwise a plain substring; a leading `!` negates.
  const matcher = (pattern) => {
    let text = pattern === undefined || pattern === null ? '' : String(pattern);
    let negate = false;
    if (text.startsWith('!')) {
      negate = true;
      text = text.slice(1);
    }
    let test;
    if (text === '' || text === '*') {
      test = () => true;
    } else {
      const re = /^\/(.+)\/([imsu]*)$/.exec(text);
      if (re) {
        const compiled = new NativeRegExp(re[1], re[2]);
        test = (subject) => compiled.test(subject);
      } else {
        test = (subject) => subject.includes(text);
      }
    }
    return negate ? (subject) => !test(String(subject)) : (subject) => test(String(subject));
  };

  // A pattern for `String.prototype.replace`: `/re/flags`, or a literal replaced everywhere.
  const replacer = (pattern) => {
    const re = /^\/(.+)\/([gimsu]*)$/.exec(String(pattern));
    if (re) return new NativeRegExp(re[1], re[2] || '');
    return new NativeRegExp(String(pattern).replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'g');
  };

  // The values `set-constant` may set. Trusted rules may also set any JSON value.
  const constant = (raw, trusted) => {
    switch (raw) {
      case 'undefined': return { value: undefined };
      case 'null': return { value: null };
      case 'false': return { value: false };
      case 'true': return { value: true };
      case "''": case '""': case 'emptyStr': return { value: '' };
      case '[]': case 'emptyArr': return { value: [] };
      case '{}': case 'emptyObj': return { value: {} };
      case 'noopFunc': return { value: function () {} };
      case 'trueFunc': return { value: function () { return true; } };
      case 'falseFunc': return { value: function () { return false; } };
      case 'throwFunc': return { value: function () { throw new Error(); } };
      case 'yes': case 'no': case 'on': case 'off': return { value: raw };
      default:
        if (/^-?\d+(?:\.\d+)?$/.test(String(raw))) {
          const number = Number(raw);
          if (Math.abs(number) <= 0x7fff || trusted) return { value: number };
          return null;
        }
        if (trusted) {
          try {
            return { value: jsonParse(raw) };
          } catch (_) {
            return { value: raw };
          }
        }
        return null;
    }
  };

  const isObject = (value) =>
    value !== null && (typeof value === 'object' || typeof value === 'function');

  // Waits for each link of `chain` (`a.b.c`) to exist, starting at `owner`, and calls `onLast`
  // with the object that holds the last property and its name. A link that is missing now is
  // trapped by an accessor until the page assigns it.
  const along = (owner, chain, onLast) => {
    const dot = chain.indexOf('.');
    if (dot === -1) {
      onLast(owner, chain);
      return;
    }
    const head = chain.slice(0, dot);
    const rest = chain.slice(dot + 1);
    let inner;
    try {
      inner = owner[head];
    } catch (_) {
      return;
    }
    if (isObject(inner)) {
      along(inner, rest, onLast);
      return;
    }
    const descriptor = getOwnPropertyDescriptor(owner, head);
    if (descriptor && descriptor.configurable === false) return;
    let current = inner;
    try {
      defineProperty(owner, head, {
        configurable: true,
        enumerable: true,
        get() {
          return current;
        },
        set(value) {
          current = value;
          if (isObject(value)) along(value, rest, onLast);
        },
      });
    } catch (_) {}
  };

  const setConstant = (trusted) => (chain, raw) => {
    if (!chain) return;
    const wanted = constant(raw, trusted);
    if (wanted === null) return;
    along(win, String(chain), (owner, prop) => {
      const descriptor = getOwnPropertyDescriptor(owner, prop);
      if (descriptor && descriptor.configurable === false) {
        try {
          owner[prop] = wanted.value;
        } catch (_) {}
        return;
      }
      try {
        defineProperty(owner, prop, {
          configurable: true,
          enumerable: descriptor ? descriptor.enumerable : true,
          get() {
            return wanted.value;
          },
          set() {},
        });
      } catch (_) {}
    });
  };

  // Paths as `json-prune` writes them: `a.b.c`, `[]` or `*` for every element or key.
  const prunePath = (object, parts, at) => {
    if (!isObject(object)) return false;
    const part = parts[at];
    const last = at === parts.length - 1;
    if (part === '[]' || part === '*') {
      let any = false;
      for (const key of keys(object)) {
        if (last) {
          delete object[key];
          any = true;
        } else if (prunePath(object[key], parts, at + 1)) {
          any = true;
        }
      }
      return any;
    }
    if (!hasOwn(object, part)) return false;
    if (last) {
      delete object[part];
      return true;
    }
    return prunePath(object[part], parts, at + 1);
  };

  const hasPath = (object, parts, at) => {
    if (at === parts.length) return true;
    if (!isObject(object)) return false;
    const part = parts[at];
    if (part === '[]' || part === '*') {
      return keys(object).some((key) => hasPath(object[key], parts, at + 1));
    }
    return hasOwn(object, part) && hasPath(object[part], parts, at + 1);
  };

  const words = (text) => String(text || '').split(/\s+/).filter((word) => word !== '');

  // Prunes `paths` from an object when every one of `needles` is in it; returns whether it did.
  const pruner = (paths, needles) => {
    const prune = words(paths).map((path) => path.split('.'));
    const need = words(needles).map((path) => path.split('.'));
    return (object) => {
      if (!isObject(object) || prune.length === 0) return false;
      if (!need.every((parts) => hasPath(object, parts, 0))) return false;
      let pruned = false;
      for (const parts of prune) {
        if (prunePath(object, parts, 0)) pruned = true;
      }
      return pruned;
    };
  };

  const stackMatches = (needle) => {
    if (!needle) return () => true;
    const test = matcher(needle);
    return () => test(new Error().stack || '');
  };

  const jsonPrune = (paths, needles, stack) => {
    const prune = pruner(paths, needles);
    const inStack = stackMatches(stack);
    JSON.parse = new NativeProxy(JSON.parse, {
      apply(target, self, args) {
        const parsed = apply(target, self, args);
        if (inStack()) prune(parsed);
        return parsed;
      },
    });
    const json = NativeResponse.prototype.json;
    NativeResponse.prototype.json = new NativeProxy(json, {
      apply(target, self, args) {
        return apply(target, self, args).then((parsed) => {
          if (inStack()) prune(parsed);
          return parsed;
        });
      },
    });
  };

  // `propsToMatch` as filter lists write it: `key:value` pairs separated by spaces, a bare
  // value is the URL; each value is a pattern.
  const requestMatcher = (props) => {
    const wanted = words(props).map((word) => {
      const colon = /^(url|method|body|mode|credentials|cache|headers):/.exec(word);
      return colon
        ? { key: colon[1], test: matcher(word.slice(colon[0].length)) }
        : { key: 'url', test: matcher(word) };
    });
    return (details) =>
      wanted.every(({ key, test }) => details[key] !== undefined && test(details[key]));
  };

  const fetchDetails = (args) => {
    const [resource, init] = args;
    const details = {};
    if (resource instanceof win.Request) {
      details.url = resource.url;
      details.method = resource.method;
    } else {
      details.url = String(resource);
    }
    if (isObject(init)) {
      for (const key of ['method', 'body', 'mode', 'credentials', 'cache']) {
        if (init[key] !== undefined) details[key] = String(init[key]);
      }
    }
    if (details.method === undefined) details.method = 'GET';
    return details;
  };

  // A response like `original` with `body` instead of its own.
  const rebuilt = (original, body) => {
    const response = new NativeResponse(body, {
      status: original.status,
      statusText: original.statusText,
      headers: original.headers,
    });
    for (const key of ['url', 'type', 'redirected']) {
      try {
        defineProperty(response, key, { value: original[key] });
      } catch (_) {}
    }
    return response;
  };

  // Rewrites the bodies of the fetches `props` matches: `edit(text)` returns the new text, or
  // `undefined` to keep the response as it came.
  const onFetch = (props, edit) => {
    const matches = requestMatcher(props);
    win.fetch = new NativeProxy(win.fetch, {
      apply(target, self, args) {
        const answer = apply(target, self, args);
        let details;
        try {
          details = fetchDetails(args);
        } catch (_) {
          return answer;
        }
        if (!matches(details)) return answer;
        return answer.then((response) =>
          response
            .clone()
            .text()
            .then((text) => {
              const edited = edit(text);
              return edited === undefined ? response : rebuilt(response, edited);
            })
            .catch(() => response),
        );
      },
    });
  };

  // Rewrites the bodies of the XMLHttpRequests `props` matches, as the page reads them.
  const onXhr = (props, edit) => {
    const matches = requestMatcher(props);
    const proto = win.XMLHttpRequest && win.XMLHttpRequest.prototype;
    if (!proto) return;
    const opened = new WeakMap();
    proto.open = new NativeProxy(proto.open, {
      apply(target, self, args) {
        opened.set(self, { method: String(args[0]).toUpperCase(), url: String(args[1]) });
        return apply(target, self, args);
      },
    });
    const cache = new WeakMap();
    const editedText = (xhr, raw) => {
      const seen = cache.get(xhr);
      if (seen && seen.raw === raw) return seen.edited;
      const edited = edit(raw);
      const result = edited === undefined ? raw : edited;
      cache.set(xhr, { raw, edited: result });
      return result;
    };
    const wrap = (prop) => {
      const descriptor = getOwnPropertyDescriptor(proto, prop);
      if (!descriptor || typeof descriptor.get !== 'function') return;
      defineProperty(proto, prop, {
        ...descriptor,
        get() {
          const raw = apply(descriptor.get, this, []);
          const details = opened.get(this);
          if (!details || this.readyState !== 4 || !matches(details)) return raw;
          if (typeof raw === 'string') return editedText(this, raw);
          if (prop === 'response' && this.responseType === 'json' && isObject(raw)) {
            try {
              return jsonParse(editedText(this, jsonStringify(raw)));
            } catch (_) {
              return raw;
            }
          }
          return raw;
        },
      });
    };
    wrap('responseText');
    wrap('response');
  };

  const jsonEdit = (paths, needles) => {
    const prune = pruner(paths, needles);
    return (text) => {
      let parsed;
      try {
        parsed = jsonParse(text);
      } catch (_) {
        return undefined;
      }
      return prune(parsed) ? jsonStringify(parsed) : undefined;
    };
  };

  const textEdit = (pattern, replacement) => {
    if (!pattern) return () => undefined;
    const re = replacer(pattern);
    const by = replacement === undefined ? '' : String(replacement);
    return (text) => {
      const edited = text.replace(re, by);
      return edited === text ? undefined : edited;
    };
  };

  // Extra arguments come as `name, value` pairs: `propsToMatch, /player/`.
  const option = (rest, name) => {
    for (let at = 0; at + 1 < rest.length; at += 2) {
      if (rest[at] === name) return rest[at + 1];
    }
    return '';
  };

  const abortOn = (onRead, onWrite) => (chain) => {
    if (!chain) return;
    along(win, String(chain), (owner, prop) => {
      let value;
      try {
        value = owner[prop];
      } catch (_) {}
      try {
        defineProperty(owner, prop, {
          configurable: true,
          get() {
            if (onRead) throw new ReferenceError(prop);
            return value;
          },
          set(next) {
            if (onWrite) throw new ReferenceError(prop);
            value = next;
          },
        });
      } catch (_) {}
    });
  };

  const abortCurrentScript = (chain, needle) => {
    if (!chain) return;
    const test = matcher(needle);
    const thisScript = document.currentScript;
    const aborts = () => {
      const script = document.currentScript;
      if (!script || script === thisScript) return false;
      const text = script.src ? script.src : script.textContent;
      return test(text || '');
    };
    along(win, String(chain), (owner, prop) => {
      const descriptor = getOwnPropertyDescriptor(owner, prop);
      if (descriptor && descriptor.configurable === false) return;
      let value = descriptor && 'value' in descriptor ? descriptor.value : undefined;
      const getter = descriptor && descriptor.get;
      const setter = descriptor && descriptor.set;
      try {
        defineProperty(owner, prop, {
          configurable: true,
          get() {
            if (aborts()) throw new ReferenceError(prop);
            return getter ? apply(getter, this, []) : value;
          },
          set(next) {
            if (aborts()) throw new ReferenceError(prop);
            if (setter) apply(setter, this, [next]);
            else value = next;
          },
        });
      } catch (_) {}
    });
  };

  // Matches a timer's delay: empty for any, `n`, or `!n` for any but `n`.
  const delayMatcher = (delay) => {
    const text = String(delay === undefined ? '' : delay);
    if (text === '' || text === '*') return () => true;
    const negate = text.startsWith('!');
    const wanted = Number(negate ? text.slice(1) : text);
    return (actual) => (Number(actual) === wanted) !== negate;
  };

  const noTimerIf = (name) => (needle, delay) => {
    const test = matcher(needle);
    const delayMatches = delayMatcher(delay);
    win[name] = new NativeProxy(win[name], {
      apply(target, self, args) {
        if (test(fnText(args[0])) && delayMatches(args[1])) {
          return apply(target, self, [function () {}, args[1]]);
        }
        return apply(target, self, args);
      },
    });
  };

  const adjustTimer = (name) => (needle, delay, boost) => {
    const test = matcher(needle);
    const wanted = delay === undefined || delay === '' ? 1000 : delay;
    const delayMatches = delay === '*' ? () => true : delayMatcher(wanted);
    let factor = Number(boost === undefined || boost === '' ? 0.05 : boost);
    if (!Number.isFinite(factor)) return;
    factor = Math.min(50, Math.max(0.001, factor));
    win[name] = new NativeProxy(win[name], {
      apply(target, self, args) {
        if (test(fnText(args[0])) && delayMatches(args[1])) {
          args[1] = Number(args[1]) * factor;
        }
        return apply(target, self, args);
      },
    });
  };

  const addEventListenerDefuser = (type, needle) => {
    const typeMatches = matcher(type);
    const test = matcher(needle);
    const proto = win.EventTarget.prototype;
    proto.addEventListener = new NativeProxy(proto.addEventListener, {
      apply(target, self, args) {
        if (typeMatches(args[0]) && test(fnText(args[1]))) return undefined;
        return apply(target, self, args);
      },
    });
  };

  const noFetchIf = (props, body) => {
    if (props === undefined) return;
    const matches = requestMatcher(props);
    win.fetch = new NativeProxy(win.fetch, {
      apply(target, self, args) {
        let details;
        try {
          details = fetchDetails(args);
        } catch (_) {
          return apply(target, self, args);
        }
        if (!matches(details)) return apply(target, self, args);
        const text = body === 'emptyObj' ? '{}' : body === 'emptyArr' ? '[]' : '';
        const response = new NativeResponse(text, { status: 200, statusText: 'OK' });
        try {
          defineProperty(response, 'url', { value: details.url });
        } catch (_) {}
        return Promise.resolve(response);
      },
    });
  };

  const noXhrIf = (props) => {
    if (props === undefined) return;
    const matches = requestMatcher(props);
    const proto = win.XMLHttpRequest && win.XMLHttpRequest.prototype;
    if (!proto) return;
    const refused = new WeakSet();
    proto.open = new NativeProxy(proto.open, {
      apply(target, self, args) {
        if (matches({ method: String(args[0]).toUpperCase(), url: String(args[1]) })) {
          refused.add(self);
        }
        return apply(target, self, args);
      },
    });
    proto.send = new NativeProxy(proto.send, {
      apply(target, self, args) {
        if (!refused.has(self)) return apply(target, self, args);
        for (const [key, value] of [
          ['readyState', 4], ['status', 200], ['statusText', 'OK'],
          ['responseText', ''], ['response', ''],
        ]) {
          try {
            defineProperty(self, key, { value, configurable: true });
          } catch (_) {}
        }
        setTimeout(() => {
          for (const type of ['readystatechange', 'load', 'loadend']) {
            self.dispatchEvent(new Event(type));
          }
        }, 1);
        return undefined;
      },
    });
  };

  const noEvalIf = (needle) => {
    const test = matcher(needle);
    win.eval = new NativeProxy(win.eval, {
      apply(target, self, args) {
        if (test(String(args[0]))) return undefined;
        return apply(target, self, args);
      },
    });
  };

  const noWindowOpenIf = (needle) => {
    const test = matcher(needle);
    win.open = new NativeProxy(win.open, {
      apply(target, self, args) {
        if (test(args[0] === undefined ? '' : String(args[0]))) return null;
        return apply(target, self, args);
      },
    });
  };

  // Runs `edit` on every element `selector` matches, now and as the page changes.
  const onElements = (selector, edit) => {
    const run = () => {
      try {
        for (const element of document.querySelectorAll(selector)) edit(element);
      } catch (_) {}
    };
    let queued = false;
    const later = () => {
      if (queued) return;
      queued = true;
      setTimeout(() => {
        queued = false;
        run();
      }, 50);
    };
    const start = () => {
      run();
      new MutationObserver(later).observe(document, {
        childList: true,
        subtree: true,
        attributes: true,
      });
    };
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', start, { once: true });
    } else {
      start();
    }
  };

  const names = (text) => String(text || '').split(/[\s|]+/).filter((name) => name !== '');

  const removeAttr = (attrs, selector) => {
    const list = names(attrs);
    if (list.length === 0) return;
    const select = selector || list.map((attr) => `[${CSS.escape(attr)}]`).join(',');
    onElements(select, (element) => {
      for (const attr of list) element.removeAttribute(attr);
    });
  };

  const removeClass = (classes, selector) => {
    const list = names(classes);
    if (list.length === 0) return;
    const select = selector || list.map((name) => `.${CSS.escape(name)}`).join(',');
    onElements(select, (element) => element.classList.remove(...list));
  };

  const SAFE_STORED = new Set([
    'undefined', 'false', 'true', 'null', '', 'emptyArr', 'emptyObj', 'yes', 'no', 'on', 'off',
    'accept', 'accepted', 'reject', 'rejected', 'allow', 'deny', 'necessary', 'required', 'ok',
    '$remove$',
  ]);

  const storedValue = (value) => {
    if (value === 'emptyArr') return '[]';
    if (value === 'emptyObj') return '{}';
    return String(value);
  };

  const setStorageItem = (which) => (key, value) => {
    if (!key) return;
    const text = value === undefined ? '' : String(value);
    if (!SAFE_STORED.has(text) && !/^\d{1,5}$/.test(text)) return;
    try {
      const storage = win[which];
      if (text === '$remove$') storage.removeItem(key);
      else storage.setItem(key, storedValue(text));
    } catch (_) {}
  };

  const setCookie = (name, value, path) => {
    if (!name) return;
    const text = value === undefined ? '' : String(value);
    if (!SAFE_STORED.has(text) && !/^\d{1,5}$/.test(text)) return;
    const where = path === 'none' ? '' : `; path=${path || '/'}`;
    try {
      document.cookie = `${encodeURIComponent(name)}=${encodeURIComponent(storedValue(text))}${where}`;
    } catch (_) {}
  };

  // Wraps the function at `chain` (`JSON.stringify`, `Object.assign`), once it exists.
  const aroundFunction = (chain, wrap) => {
    if (!chain) return;
    along(win, String(chain), (owner, prop) => {
      const original = owner[prop];
      if (typeof original !== 'function') return;
      try {
        owner[prop] = new NativeProxy(original, { apply: wrap });
      } catch (_) {}
    });
  };

  const pruneInbound = (chain, position, paths, needles) => {
    const prune = pruner(paths, needles);
    const at = Number(position) || 1;
    aroundFunction(chain, (target, self, args) => {
      if (at >= 1 && at <= args.length && isObject(args[at - 1])) {
        try {
          args[at - 1] = jsonParse(jsonStringify(args[at - 1]));
          prune(args[at - 1]);
        } catch (_) {}
      }
      return apply(target, self, args);
    });
  };

  const pruneOutbound = (chain, paths, needles) => {
    const prune = pruner(paths, needles);
    aroundFunction(chain, (target, self, args) => {
      const result = apply(target, self, args);
      prune(result);
      return result;
    });
  };

  const replaceOutbound = (chain, pattern, replacement) => {
    const edit = textEdit(pattern, replacement);
    aroundFunction(chain, (target, self, args) => {
      const result = apply(target, self, args);
      if (typeof result !== 'string') return result;
      const edited = edit(result);
      return edited === undefined ? result : edited;
    });
  };

  const library = {
    'set-constant': setConstant(false),
    'trusted-set-constant': setConstant(true),
    'json-prune': jsonPrune,
    'json-prune-fetch-response': (paths, needles, ...rest) =>
      onFetch(option(rest, 'propsToMatch'), jsonEdit(paths, needles)),
    'json-prune-xhr-response': (paths, needles, ...rest) =>
      onXhr(option(rest, 'propsToMatch'), jsonEdit(paths, needles)),
    'trusted-replace-fetch-response': (pattern, replacement, props) =>
      onFetch(props, textEdit(pattern, replacement)),
    'trusted-replace-xhr-response': (pattern, replacement, props) =>
      onXhr(props, textEdit(pattern, replacement)),
    'abort-on-property-read': abortOn(true, false),
    'abort-on-property-write': abortOn(false, true),
    'abort-current-script': abortCurrentScript,
    'no-setTimeout-if': noTimerIf('setTimeout'),
    'no-setInterval-if': noTimerIf('setInterval'),
    'adjust-setTimeout': adjustTimer('setTimeout'),
    'adjust-setInterval': adjustTimer('setInterval'),
    'addEventListener-defuser': addEventListenerDefuser,
    'no-fetch-if': noFetchIf,
    'no-xhr-if': noXhrIf,
    'noeval-if': noEvalIf,
    'noeval': () => noEvalIf(''),
    'no-window-open-if': noWindowOpenIf,
    'remove-attr': removeAttr,
    'remove-class': removeClass,
    'set-cookie': setCookie,
    'set-local-storage-item': setStorageItem('localStorage'),
    'set-session-storage-item': setStorageItem('sessionStorage'),
    'trusted-prune-inbound-object': pruneInbound,
    'trusted-prune-outbound-object': pruneOutbound,
    'trusted-replace-outbound-text': replaceOutbound,
  };

  return (name, args) => {
    const scriptlet = library[name];
    if (typeof scriptlet !== 'function') return;
    try {
      scriptlet(...args);
    } catch (_) {}
  };
})()
