//! Fixed isolated-world DOM locator, provenance signal extraction, state observation, and
//! event-driven wait helpers for [`super::AgentAutomation`].
//!
//! Every script executes in Quickie's isolated `WKContentWorld` using the existing `scripts`
//! evaluation path and `WKWebView::callAsyncJavaScript`. No caller-supplied JavaScript is ever
//! accepted or evaluated.

#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::time::Duration;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSDictionary, NSError, NSNumber, NSString};
use objc2_web_kit::WKWebView;
use serde_json::{Value, json};

use crate::automation::{AutomationError, MAX_VIEWPORT_COORD, ViewportPoint};
use crate::page::scripts;

const MAX_LOCATE_RESULTS_CAP: usize = 50;
const MAX_NAME_BYTES: usize = 256;
const MAX_ROLE_BYTES: usize = 64;
const MAX_TAG_BYTES: usize = 64;

/// Typed element locator query executed in Quickie's isolated content world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocatorQuery {
    /// Match elements by CSS selector.
    Css {
        /// Validated CSS selector string.
        selector: String,
    },
    /// Match elements by visible text.
    Text {
        /// Text to match.
        text: String,
        /// Exact match when `true`, case-insensitive substring when `false`.
        exact: bool,
    },
    /// Match elements by accessibility/semantic role and optional accessible name.
    Role {
        /// Target role name (case-insensitive).
        role: String,
        /// Optional accessible name filter.
        name: Option<String>,
        /// Exact name match when `true`, case-insensitive substring when `false`.
        exact: bool,
    },
}

/// CSS-pixel bounding box returned from isolated-world DOM geometry queries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DomBounds {
    /// Horizontal offset relative to the visual viewport.
    pub x: f64,
    /// Vertical offset relative to the visual viewport.
    pub y: f64,
    /// Element width in CSS pixels.
    pub width: f64,
    /// Element height in CSS pixels.
    pub height: f64,
}

/// Typed provenance and containment signals for a located or focused DOM element.
#[derive(Clone, Debug, Default, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag represents an independent typed DOM containment signal"
)]
pub struct ElementSignals {
    /// Whether the referenced node is still connected to the active document.
    pub connected: bool,
    /// Whether the element has non-zero visible geometry.
    pub visible: bool,
    /// Whether activating the element submits a form (`<button type=submit>`, `<input type=submit>`).
    pub is_submit_control: bool,
    /// Whether the element belongs to a `<form>` (relevant for `Enter` submission).
    pub in_form: bool,
    /// Whether the element is an `<input type="file">`.
    pub is_file_input: bool,
    /// Whether the element accepts text input (`<input>`, `<textarea>`, `contenteditable`).
    pub is_editable: bool,
    /// Whether the element is a `<select>`.
    pub is_select: bool,
    /// Resolved href of the element or its closest anchor ancestor, if any.
    pub link_url: Option<String>,
    /// Whether the closest anchor ancestor targets a new browsing context (`_blank`).
    pub link_opens_new_window: bool,
    /// Validated viewport center coordinate suitable for trusted mouse/wheel input.
    pub action_point: Option<ViewportPoint>,
}

/// One element match returned by [`super::AgentAutomation::locate_elements`].
#[derive(Clone, Debug, PartialEq)]
pub struct LocatedDomElement {
    /// Opaque document-scoped element handle stored in the isolated world.
    pub element_id: String,
    /// Accessibility or implicit semantic role.
    pub role: String,
    /// Bounded accessible name or visible label.
    pub name: Option<String>,
    /// Lowercase HTML tag name.
    pub tag: Option<String>,
    /// Bounding box in CSS viewport pixels.
    pub bounds: Option<DomBounds>,
    /// Typed containment/provenance signals for this element.
    pub signals: ElementSignals,
}

/// Bounded locator result from Quickie's isolated content world.
#[derive(Clone, Debug, PartialEq)]
pub struct LocatorSnapshot {
    /// Ordered matched elements.
    pub elements: Vec<LocatedDomElement>,
    /// Whether more elements matched than `max_results`.
    pub truncated: bool,
}

/// Bounded post-action page state observed in Quickie's isolated content world.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObservedPageState {
    /// Accessibility role of `document.activeElement`, if any.
    pub focused_role: Option<String>,
    /// Character count of the focused input's value (never raw text).
    pub value_length: Option<usize>,
    /// Horizontal viewport scroll offset in CSS pixels.
    pub scroll_x: Option<i32>,
    /// Vertical viewport scroll offset in CSS pixels.
    pub scroll_y: Option<i32>,
}

/// Event-driven wait condition evaluated in Quickie's isolated content world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitQuery {
    /// Wait until at least one element matching `LocatorQuery` is visible.
    LocatorVisible(LocatorQuery),
    /// Wait until no element matching `LocatorQuery` is visible.
    LocatorHidden(LocatorQuery),
    /// Wait until visible text appears in the document.
    TextPresent {
        /// Text to match.
        text: String,
        /// Exact match when `true`, substring when `false`.
        exact: bool,
    },
    /// Wait until `DOMContentLoaded` (`readyState !== "loading"`).
    DomContentLoaded,
    /// Wait until `load` (`readyState === "complete"`).
    Load,
}

/// Outcome of [`super::AgentAutomation::wait_for_condition`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The condition matched before the deadline.
    Matched,
    /// The deadline elapsed before the condition matched.
    TimedOut,
}

/// Shared isolated-world helper preamble: role inference, accessible name, visibility, signals,
/// and per-document opaque node registry (stored only inside Quickie's isolated `WKContentWorld`).
const ISOLATED_HELPERS_JS: &str = r#"
const ensureRegistry = () => {
  if (!globalThis.__quickieAgentReg || globalThis.__quickieAgentReg.doc !== document) {
    globalThis.__quickieAgentReg = {
      doc: document,
      seq: 0,
      byId: new Map(),
      byEl: new WeakMap(),
    };
  }
  return globalThis.__quickieAgentReg;
};
const idFor = (el) => {
  const reg = ensureRegistry();
  let id = reg.byEl.get(el);
  if (!id) {
    reg.seq += 1;
    id = 'el-' + reg.seq;
    reg.byEl.set(el, id);
    reg.byId.set(id, el);
  }
  return id;
};
const clean = (s, maxLen) => {
  const t = String(s || '').replace(/\s+/g, ' ').trim();
  return t.length > maxLen ? t.slice(0, maxLen) : t;
};
const implicitRole = (el) => {
  const explicit = el.getAttribute && el.getAttribute('role');
  if (explicit) return clean(explicit, 64).toLowerCase();
  const tag = String(el.tagName || '').toLowerCase();
  if (tag === 'button') return 'button';
  if (tag === 'a' && el.hasAttribute('href')) return 'link';
  if (tag === 'textarea') return 'textbox';
  if (tag === 'select') return 'combobox';
  if (tag === 'option') return 'option';
  if (tag === 'img') return 'img';
  if (tag === 'nav') return 'navigation';
  if (tag === 'main') return 'main';
  if (tag === 'form') return 'form';
  if (tag === 'dialog') return 'dialog';
  if (tag === 'table') return 'table';
  if (tag === 'tr') return 'row';
  if (tag === 'td' || tag === 'th') return 'cell';
  if (tag === 'ul' || tag === 'ol') return 'list';
  if (tag === 'li') return 'listitem';
  if (/^h[1-6]$/.test(tag)) return 'heading';
  if (tag === 'input') {
    const t = String(el.type || 'text').toLowerCase();
    if (t === 'button' || t === 'submit' || t === 'reset' || t === 'image') return 'button';
    if (t === 'checkbox') return 'checkbox';
    if (t === 'radio') return 'radio';
    if (t === 'search') return 'searchbox';
    return 'textbox';
  }
  return 'generic';
};
const accessibleName = (el) => {
  const aria = el.getAttribute && el.getAttribute('aria-label');
  if (aria && aria.trim()) return clean(aria, 256);
  const labelledBy = el.getAttribute && el.getAttribute('aria-labelledby');
  if (labelledBy) {
    const parts = labelledBy.trim().split(/\s+/).map((id) => {
      const ref = document.getElementById(id);
      return ref ? (ref.innerText || ref.textContent || '') : '';
    }).filter(Boolean);
    if (parts.length) return clean(parts.join(' '), 256);
  }
  if (el.id) {
    const lbl = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
    if (lbl) {
      const t = clean(lbl.innerText || lbl.textContent || '', 256);
      if (t) return t;
    }
  }
  const alt = el.getAttribute && el.getAttribute('alt');
  if (alt && alt.trim()) return clean(alt, 256);
  const title = el.getAttribute && el.getAttribute('title');
  if (title && title.trim()) return clean(title, 256);
  const tag = String(el.tagName || '').toLowerCase();
  if (tag === 'input') {
    const t = String(el.type || 'text').toLowerCase();
    if ((t === 'button' || t === 'submit' || t === 'reset') && el.value) {
      return clean(el.value, 256);
    }
    if (el.placeholder) return clean(el.placeholder, 256);
  }
  const text = clean(el.innerText || el.textContent || '', 256);
  return text || null;
};
const isVisible = (el) => {
  if (!el || !el.isConnected) return false;
  const style = getComputedStyle(el);
  if (!style || style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') {
    return false;
  }
  const r = el.getBoundingClientRect();
  return Number.isFinite(r.width) && Number.isFinite(r.height) && r.width > 0 && r.height > 0;
};
const extractSignals = (el) => {
  if (!el || !el.isConnected) {
    return { connected: false };
  }
  const r = el.getBoundingClientRect();
  const visible = isVisible(el);
  const tag = String(el.tagName || '').toLowerCase();
  const inputType = tag === 'input' ? String(el.type || 'text').toLowerCase() : '';
  const form = el.form || (el.closest ? el.closest('form') : null);
  const buttonType = tag === 'button' ? String(el.getAttribute('type') || 'submit').toLowerCase() : '';
  const isSubmit = (tag === 'input' && (inputType === 'submit' || inputType === 'image'))
    || (tag === 'button' && !!form && buttonType === 'submit');
  const isFile = tag === 'input' && inputType === 'file';
  const nonTextInputs = new Set(['button', 'checkbox', 'color', 'file', 'hidden', 'image', 'radio', 'range', 'reset', 'submit']);
  const isEditable = !!el.isContentEditable || tag === 'textarea' || (tag === 'input' && !nonTextInputs.has(inputType));
  const isSelect = tag === 'select';
  const anchor = el.closest ? el.closest('a[href], area[href]') : null;
  let linkUrl = null;
  let linkBlank = false;
  if (anchor && anchor.href) {
    linkUrl = String(anchor.href);
    const target = String(anchor.getAttribute('target') || '').toLowerCase();
    linkBlank = target === '_blank';
  }
  const cx = r.left + r.width / 2;
  const cy = r.top + r.height / 2;
  return {
    connected: true,
    visible,
    is_submit_control: isSubmit,
    in_form: !!form,
    is_file_input: isFile,
    is_editable: isEditable,
    is_select: isSelect,
    link_url: linkUrl,
    link_opens_new_window: linkBlank,
    bounds: { x: r.left, y: r.top, width: r.width, height: r.height },
    point: visible ? { x: cx, y: cy } : null,
  };
};
const matchElements = (q) => {
  const kind = q.kind;
  if (kind === 'css') {
    return Array.from(document.querySelectorAll(q.selector));
  }
  const all = Array.from(document.querySelectorAll('*'));
  if (kind === 'text') {
    const needle = q.exact ? q.text.trim() : q.text.trim().toLowerCase();
    return all.filter((el) => {
      if (!isVisible(el)) return false;
      const own = clean(el.innerText || '', 1024);
      if (!own) return false;
      const hay = q.exact ? own : own.toLowerCase();
      const matched = q.exact ? hay === needle : hay.includes(needle);
      if (!matched) return false;
      for (const child of el.children || []) {
        const ct = clean(child.innerText || '', 1024);
        const chay = q.exact ? ct : ct.toLowerCase();
        if (q.exact ? chay === needle : chay.includes(needle)) return false;
      }
      return true;
    });
  }
  if (kind === 'role') {
    const wantRole = q.role.trim().toLowerCase();
    const wantName = q.name ? (q.exact ? q.name.trim() : q.name.trim().toLowerCase()) : null;
    return all.filter((el) => {
      if (!isVisible(el)) return false;
      if (implicitRole(el) !== wantRole) return false;
      if (wantName === null) return true;
      const nm = accessibleName(el) || '';
      const hay = q.exact ? nm : nm.toLowerCase();
      return q.exact ? hay === wantName : hay.includes(wantName);
    });
  }
  return [];
};
"#;

pub(crate) fn locate(
    view: &WKWebView,
    mtm: MainThreadMarker,
    query: &LocatorQuery,
    max_results: usize,
    done: impl FnOnce(Result<LocatorSnapshot, AutomationError>) + 'static,
) -> Result<(), AutomationError> {
    let limit = max_results.clamp(1, MAX_LOCATE_RESULTS_CAP);
    let script = build_locate_script(query, limit)?;
    let world = scripts::world(mtm);
    scripts::evaluate(view, &world, None, &script, move |raw| {
        done(parse_locate_reply(&raw, limit));
    });
    Ok(())
}

pub(crate) fn resolve_element(
    view: &WKWebView,
    mtm: MainThreadMarker,
    element_id: &str,
    scroll_into_view: bool,
    done: impl FnOnce(Result<ElementSignals, AutomationError>) + 'static,
) -> Result<(), AutomationError> {
    let script = build_resolve_script(element_id, scroll_into_view)?;
    let world = scripts::world(mtm);
    scripts::evaluate(view, &world, None, &script, move |raw| {
        done(parse_signals_reply(&raw));
    });
    Ok(())
}

pub(crate) fn inspect_focused(
    view: &WKWebView,
    mtm: MainThreadMarker,
    done: impl FnOnce(Result<ElementSignals, AutomationError>) + 'static,
) {
    let script = build_inspect_focused_script();
    let world = scripts::world(mtm);
    scripts::evaluate(view, &world, None, &script, move |raw| {
        done(parse_signals_reply(&raw));
    });
}

pub(crate) fn select_option(
    view: &WKWebView,
    mtm: MainThreadMarker,
    element_id: &str,
    by_value: Option<&str>,
    by_label: Option<&str>,
    done: impl FnOnce(Result<(), AutomationError>) + 'static,
) -> Result<(), AutomationError> {
    let script = build_select_option_script(element_id, by_value, by_label)?;
    let world = scripts::world(mtm);
    scripts::evaluate(view, &world, None, &script, move |raw| {
        done(parse_select_option_reply(&raw));
    });
    Ok(())
}

pub(crate) fn observe_state(
    view: &WKWebView,
    mtm: MainThreadMarker,
    done: impl FnOnce(Result<ObservedPageState, AutomationError>) + 'static,
) {
    let script = build_observe_state_script();
    let world = scripts::world(mtm);
    scripts::evaluate(view, &world, None, &script, move |raw| {
        done(parse_observed_state_reply(&raw));
    });
}

pub(crate) fn wait_for_condition(
    view: &WKWebView,
    mtm: MainThreadMarker,
    query: &WaitQuery,
    timeout: Duration,
    done: impl FnOnce(Result<WaitOutcome, AutomationError>) + 'static,
) -> Result<(), AutomationError> {
    if timeout.is_zero() {
        return Err(AutomationError::InvalidCommand);
    }
    let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    let query_json = serde_json::to_string(&encode_wait_query(query))
        .map_err(|_| AutomationError::InvalidCommand)?;
    let body = build_wait_async_body();

    let query_arg = NSString::from_str(&query_json);
    let timeout_arg = NSNumber::new_u32(timeout_ms);
    let values: [&AnyObject; 2] = [&query_arg, &timeout_arg];
    let arguments: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(
        &[
            &*NSString::from_str("queryJson"),
            &*NSString::from_str("timeoutMs"),
        ],
        &values,
    );

    let done = Cell::new(Some(done));
    let block = RcBlock::new(move |value: *mut AnyObject, _error: *mut NSError| {
        // SAFETY: WebKit passes a valid object or null for the duration of this main-thread callback.
        let answer = unsafe { value.as_ref() }
            .and_then(AnyObject::downcast_ref::<NSString>)
            .map_or_else(String::new, NSString::to_string);
        if let Some(done) = done.take() {
            done(parse_wait_reply(&answer));
        }
    });

    let world = scripts::world(mtm);
    // SAFETY: `body` is a fixed async function body using only its two typed arguments, targeting
    // the main frame (`None`) in Quickie's isolated content world; WebKit invokes `block` once on
    // the main thread.
    unsafe {
        view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(&body),
            Some(&arguments),
            None,
            &world,
            Some(&block),
        );
    }
    Ok(())
}

pub(crate) fn build_locate_script(
    query: &LocatorQuery,
    max_results: usize,
) -> Result<String, AutomationError> {
    let query_json = serde_json::to_string(&encode_locator_query(query))
        .map_err(|_| AutomationError::InvalidCommand)?;
    Ok(format!(
        r"(() => {{
{ISOLATED_HELPERS_JS}
try {{
  const matches = matchElements({query_json});
  const limit = {max_results};
  const truncated = matches.length > limit;
  const out = matches.slice(0, limit).map((el) => {{
    const sig = extractSignals(el);
    return {{
      element_id: idFor(el),
      role: implicitRole(el),
      name: accessibleName(el),
      tag: String(el.tagName || '').toLowerCase() || null,
      bounds: sig.bounds || null,
      signals: sig,
    }};
  }});
  return JSON.stringify({{ elements: out, truncated }});
}} catch (_) {{
  return JSON.stringify({{ error: 'invalid_locator' }});
}}
}})()"
    ))
}

pub(crate) fn build_resolve_script(
    element_id: &str,
    scroll_into_view: bool,
) -> Result<String, AutomationError> {
    let id_json = serde_json::to_string(element_id).map_err(|_| AutomationError::InvalidCommand)?;
    let scroll_js = if scroll_into_view {
        "if (el.scrollIntoView) el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });"
    } else {
        ""
    };
    Ok(format!(
        r"(() => {{
{ISOLATED_HELPERS_JS}
const reg = ensureRegistry();
const el = reg.byId.get({id_json});
if (!el || !el.isConnected) return JSON.stringify({{ connected: false }});
{scroll_js}
return JSON.stringify(extractSignals(el));
}})()"
    ))
}

pub(crate) fn build_inspect_focused_script() -> String {
    format!(
        r"(() => {{
{ISOLATED_HELPERS_JS}
let el = document.activeElement;
while (el && el.shadowRoot && el.shadowRoot.activeElement) el = el.shadowRoot.activeElement;
if (!el || el === document.body || el === document.documentElement) {{
  return JSON.stringify({{ connected: false }});
}}
return JSON.stringify(extractSignals(el));
}})()"
    )
}

pub(crate) fn build_select_option_script(
    element_id: &str,
    by_value: Option<&str>,
    by_label: Option<&str>,
) -> Result<String, AutomationError> {
    let id_json = serde_json::to_string(element_id).map_err(|_| AutomationError::InvalidCommand)?;
    let val_json = serde_json::to_string(&by_value).map_err(|_| AutomationError::InvalidCommand)?;
    let lbl_json = serde_json::to_string(&by_label).map_err(|_| AutomationError::InvalidCommand)?;
    Ok(format!(
        r"(() => {{
{ISOLATED_HELPERS_JS}
const reg = ensureRegistry();
const el = reg.byId.get({id_json});
if (!el || !el.isConnected) return JSON.stringify({{ error: 'stale' }});
if (String(el.tagName || '').toLowerCase() !== 'select') return JSON.stringify({{ error: 'not_select' }});
const byVal = {val_json};
const byLbl = {lbl_json};
const options = Array.from(el.options || []);
let picked = null;
for (const opt of options) {{
  if (byVal !== null && opt.value === byVal) {{ picked = opt; break; }}
  if (byLbl !== null && clean(opt.label || opt.textContent || '', 4096) === byLbl) {{ picked = opt; break; }}
}}
if (!picked) return JSON.stringify({{ error: 'option_not_found' }});
el.focus();
picked.selected = true;
el.dispatchEvent(new Event('input', {{ bubbles: true }}));
el.dispatchEvent(new Event('change', {{ bubbles: true }}));
return JSON.stringify({{ ok: true }});
}})()"
    ))
}

pub(crate) fn build_observe_state_script() -> String {
    format!(
        r"(() => {{
{ISOLATED_HELPERS_JS}
let el = document.activeElement;
while (el && el.shadowRoot && el.shadowRoot.activeElement) el = el.shadowRoot.activeElement;
const hasFocus = !!el && el !== document.body && el !== document.documentElement;
const focused_role = hasFocus ? implicitRole(el) : null;
let value_length = null;
if (hasFocus) {{
  const tag = String(el.tagName || '').toLowerCase();
  if (tag === 'input' || tag === 'textarea') {{
    value_length = String(el.value || '').length;
  }} else if (el.isContentEditable) {{
    value_length = String(el.innerText || '').length;
  }}
}}
return JSON.stringify({{
  focused_role,
  value_length,
  scroll_x: Math.round(window.scrollX || 0),
  scroll_y: Math.round(window.scrollY || 0),
}});
}})()"
    )
}

pub(crate) fn build_wait_async_body() -> String {
    format!(
        r"
{ISOLATED_HELPERS_JS}
const q = JSON.parse(queryJson);
const check = () => {{
  if (q.kind === 'dom_content_loaded') {{
    return document.readyState !== 'loading';
  }}
  if (q.kind === 'load') {{
    return document.readyState === 'complete';
  }}
  if (q.kind === 'locator_visible') {{
    return matchElements(q.locator).some((el) => isVisible(el));
  }}
  if (q.kind === 'locator_hidden') {{
    return !matchElements(q.locator).some((el) => isVisible(el));
  }}
  if (q.kind === 'text_present') {{
    const root = document.body || document.documentElement;
    const raw = clean(root ? (root.innerText || '') : '', 65536);
    const needle = q.exact ? q.text.trim() : q.text.trim().toLowerCase();
    const hay = q.exact ? raw : raw.toLowerCase();
    return q.exact ? hay === needle : hay.includes(needle);
  }}
  return false;
}};
try {{
  if (check()) return 'matched';
}} catch (_) {{
  return 'invalid';
}}
return await new Promise((resolve) => {{
  let settled = false;
  let observer = null;
  let timer = null;
  const finish = (status) => {{
    if (settled) return;
    settled = true;
    if (observer) observer.disconnect();
    if (timer !== null) clearTimeout(timer);
    document.removeEventListener('DOMContentLoaded', onEvent);
    window.removeEventListener('load', onEvent);
    resolve(status);
  }};
  const onEvent = () => {{
    try {{
      if (check()) finish('matched');
    }} catch (_) {{
      finish('invalid');
    }}
  }};
  if (q.kind === 'dom_content_loaded') {{
    document.addEventListener('DOMContentLoaded', onEvent, {{ once: true }});
  }} else if (q.kind === 'load') {{
    window.addEventListener('load', onEvent, {{ once: true }});
  }} else {{
    const target = document.documentElement || document;
    observer = new MutationObserver(onEvent);
    observer.observe(target, {{ childList: true, subtree: true, attributes: true, characterData: true }});
  }}
  timer = setTimeout(() => finish('timed_out'), timeoutMs);
}});
"
    )
}

fn encode_locator_query(query: &LocatorQuery) -> Value {
    match query {
        LocatorQuery::Css { selector } => json!({ "kind": "css", "selector": selector }),
        LocatorQuery::Text { text, exact } => {
            json!({ "kind": "text", "text": text, "exact": exact })
        }
        LocatorQuery::Role { role, name, exact } => {
            json!({ "kind": "role", "role": role, "name": name, "exact": exact })
        }
    }
}

fn encode_wait_query(query: &WaitQuery) -> Value {
    match query {
        WaitQuery::LocatorVisible(locator) => {
            json!({ "kind": "locator_visible", "locator": encode_locator_query(locator) })
        }
        WaitQuery::LocatorHidden(locator) => {
            json!({ "kind": "locator_hidden", "locator": encode_locator_query(locator) })
        }
        WaitQuery::TextPresent { text, exact } => {
            json!({ "kind": "text_present", "text": text, "exact": exact })
        }
        WaitQuery::DomContentLoaded => json!({ "kind": "dom_content_loaded" }),
        WaitQuery::Load => json!({ "kind": "load" }),
    }
}

pub(crate) fn parse_locate_reply(
    raw: &str,
    limit: usize,
) -> Result<LocatorSnapshot, AutomationError> {
    if raw.is_empty() {
        return Err(AutomationError::CommandFailed);
    }
    let val: Value = serde_json::from_str(raw).map_err(|_| AutomationError::InvalidReply)?;
    if let Some(error) = val.get("error").and_then(Value::as_str) {
        return match error {
            "invalid_locator" => Err(AutomationError::InvalidCommand),
            _ => Err(AutomationError::CommandFailed),
        };
    }
    let arr = val
        .get("elements")
        .and_then(Value::as_array)
        .ok_or(AutomationError::InvalidReply)?;
    let mut elements = Vec::with_capacity(arr.len().min(limit));
    for item in arr.iter().take(limit) {
        let Some(element_id) = item
            .get("element_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let role = item
            .get("role")
            .and_then(Value::as_str)
            .map_or_else(|| "generic".to_owned(), |s| truncate_str(s, MAX_ROLE_BYTES));
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| truncate_str(s, MAX_NAME_BYTES));
        let tag = item
            .get("tag")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| truncate_str(s, MAX_TAG_BYTES));
        let bounds = item.get("bounds").and_then(parse_bounds);
        let signals = item
            .get("signals")
            .map_or_default(parse_signals_value);
        elements.push(LocatedDomElement {
            element_id: element_id.to_owned(),
            role,
            name,
            tag,
            bounds,
            signals,
        });
    }
    let truncated = val
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || arr.len() > limit;
    Ok(LocatorSnapshot {
        elements,
        truncated,
    })
}

pub(crate) fn parse_signals_reply(raw: &str) -> Result<ElementSignals, AutomationError> {
    if raw.is_empty() {
        return Err(AutomationError::CommandFailed);
    }
    let val: Value = serde_json::from_str(raw).map_err(|_| AutomationError::InvalidReply)?;
    Ok(parse_signals_value(&val))
}

fn parse_signals_value(val: &Value) -> ElementSignals {
    let connected = val
        .get("connected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !connected {
        return ElementSignals::default();
    }
    let action_point = val.get("point").and_then(parse_viewport_point);
    ElementSignals {
        connected: true,
        visible: val.get("visible").and_then(Value::as_bool).unwrap_or(false),
        is_submit_control: val
            .get("is_submit_control")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        in_form: val.get("in_form").and_then(Value::as_bool).unwrap_or(false),
        is_file_input: val
            .get("is_file_input")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_editable: val
            .get("is_editable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_select: val
            .get("is_select")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        link_url: val
            .get("link_url")
            .and_then(Value::as_str)
            .map(str::to_owned),
        link_opens_new_window: val
            .get("link_opens_new_window")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        action_point,
    }
}

pub(crate) fn parse_select_option_reply(raw: &str) -> Result<(), AutomationError> {
    if raw.is_empty() {
        return Err(AutomationError::CommandFailed);
    }
    let val: Value = serde_json::from_str(raw).map_err(|_| AutomationError::InvalidReply)?;
    if val.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    match val.get("error").and_then(Value::as_str) {
        Some("stale") => Err(AutomationError::Disconnected),
        _ => Err(AutomationError::CommandFailed),
    }
}

pub(crate) fn parse_observed_state_reply(raw: &str) -> Result<ObservedPageState, AutomationError> {
    if raw.is_empty() {
        return Err(AutomationError::CommandFailed);
    }
    let val: Value = serde_json::from_str(raw).map_err(|_| AutomationError::InvalidReply)?;
    let focused_role = val
        .get("focused_role")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| truncate_str(s, MAX_ROLE_BYTES));
    let value_length = val
        .get("value_length")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok());
    let scroll_x = val
        .get("scroll_x")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok());
    let scroll_y = val
        .get("scroll_y")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok());
    Ok(ObservedPageState {
        focused_role,
        value_length,
        scroll_x,
        scroll_y,
    })
}

pub(crate) fn parse_wait_reply(raw: &str) -> Result<WaitOutcome, AutomationError> {
    match raw {
        "matched" => Ok(WaitOutcome::Matched),
        "timed_out" => Ok(WaitOutcome::TimedOut),
        "invalid" => Err(AutomationError::InvalidCommand),
        _ => Err(AutomationError::CommandFailed),
    }
}

fn parse_bounds(val: &Value) -> Option<DomBounds> {
    let x = val.get("x")?.as_f64()?;
    let y = val.get("y")?.as_f64()?;
    let width = val.get("width")?.as_f64()?;
    let height = val.get("height")?.as_f64()?;
    if x.is_finite()
        && y.is_finite()
        && width.is_finite()
        && height.is_finite()
        && x.abs() <= MAX_VIEWPORT_COORD
        && y.abs() <= MAX_VIEWPORT_COORD
        && (0.0..=MAX_VIEWPORT_COORD).contains(&width)
        && (0.0..=MAX_VIEWPORT_COORD).contains(&height)
    {
        Some(DomBounds {
            x,
            y,
            width,
            height,
        })
    } else {
        None
    }
}

fn parse_viewport_point(val: &Value) -> Option<ViewportPoint> {
    let x = val.get("x")?.as_f64()?;
    let y = val.get("y")?.as_f64()?;
    if x.is_finite() && y.is_finite() && x.abs() <= MAX_VIEWPORT_COORD && y.abs() <= MAX_VIEWPORT_COORD
    {
        Some(ViewportPoint { x, y })
    } else {
        None
    }
}

fn truncate_str(raw: &str, max_bytes: usize) -> String {
    if raw.len() <= max_bytes {
        return raw.to_owned();
    }
    let end = raw.floor_char_boundary(max_bytes);
    raw[..end].to_owned()
}
