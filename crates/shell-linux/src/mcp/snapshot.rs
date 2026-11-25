//! Bounded isolated-world DOM and visible-text snapshots aligned with `AppKit` semantics.

#![cfg_attr(
    target_os = "linux",
    expect(
        unsafe_code,
        reason = "WebKitGTK isolated-world JavaScript evaluation and GCancellable FFI"
    )
)]

#[cfg(target_os = "linux")]
use std::ffi::{CStr, CString, c_char};
#[cfg(target_os = "linux")]
use std::ptr;

use quickie_application::DiagnosticSnapshot;
use quickie_application::mcp::{SnapshotKind, SnapshotRequest};
use quickie_mcp::{ToolError, ToolErrorCode};
use serde_json::Value;

/// Builds the fixed application-owned isolated-world script for `request`.
///
/// Mirrors `crates/adapters/engine-webkit/src/page/observation/cocoa/snapshot.rs` so Linux and
/// macOS produce identical bounded DOM and visible-text semantics and reject
/// [`SnapshotKind::Accessibility`] until native accessibility tree data is wired.
pub(crate) fn build_script(request: &SnapshotRequest) -> Result<String, ToolError> {
    if request.kind() == SnapshotKind::Accessibility {
        return Err(ToolError::new(
            ToolErrorCode::UnsupportedCapability,
            "native accessibility snapshots are unavailable",
        ));
    }
    let selector_json = match request.selector() {
        Some(selector) => serde_json::to_string(selector).map_err(|_| {
            ToolError::new(ToolErrorCode::InvalidParams, "invalid snapshot selector")
        })?,
        None => "null".to_owned(),
    };

    match request.kind() {
        SnapshotKind::Text => Ok(build_text_script(&selector_json, request.max_bytes())),
        SnapshotKind::Dom => Ok(build_dom_script(
            &selector_json,
            request.max_nodes(),
            request.max_depth(),
            request.max_bytes(),
        )),
        SnapshotKind::Accessibility => Err(ToolError::new(
            ToolErrorCode::UnsupportedCapability,
            "native accessibility snapshots are unavailable",
        )),
    }
}

fn build_text_script(selector_json: &str, max_bytes: usize) -> String {
    format!(
        r"(() => {{
  const selector = {selector_json};
  const maxBytes = {max_bytes};
  let root = null;
  if (selector !== null) {{
    try {{
      root = document.querySelector(selector);
    }} catch (_) {{
      return JSON.stringify({{ error: 'invalid_selector' }});
    }}
    if (!root) return JSON.stringify({{ error: 'selector_not_found' }});
  }} else {{
    root = document.body || document.documentElement;
  }}
  const raw = root ? String(root.innerText || '') : '';
  const enc = new TextEncoder();
  let out = '';
  let used = 0;
  let truncated = false;
  for (const ch of raw) {{
    const len = enc.encode(ch).length;
    if (used + len > maxBytes) {{
      truncated = true;
      break;
    }}
    out += ch;
    used += len;
  }}
  return JSON.stringify({{ data: out, truncated }});
}})()"
    )
}

fn build_dom_script(
    selector_json: &str,
    max_nodes: usize,
    max_depth: usize,
    max_bytes: usize,
) -> String {
    format!(
        r#"(() => {{
  const selector = {selector_json};
  const maxNodes = {max_nodes};
  const maxDepth = {max_depth};
  const maxBytes = {max_bytes};
  let root = null;
  if (selector !== null) {{
    try {{
      root = document.querySelector(selector);
    }} catch (_) {{
      return JSON.stringify({{ error: 'invalid_selector' }});
    }}
    if (!root) return JSON.stringify({{ error: 'selector_not_found' }});
  }} else {{
    root = document.documentElement || document.body;
  }}
  if (!root) return JSON.stringify({{ data: '', truncated: false }});
  const allowedAttrs = ['id', 'class', 'role', 'aria-label', 'type', 'name'];
  const enc = new TextEncoder();
  let nodes = 0;
  let bytes = 0;
  let truncated = false;
  const lines = [];
  const pushLine = (line) => {{
    const len = enc.encode(line).length + (lines.length ? 1 : 0);
    if (bytes + len > maxBytes) {{
      truncated = true;
      return false;
    }}
    lines.push(line);
    bytes += len;
    return true;
  }};
  const walk = (el, depth) => {{
    if (truncated) return;
    if (nodes >= maxNodes) {{
      truncated = true;
      return;
    }}
    nodes += 1;
    const indent = '  '.repeat(depth);
    let attrs = '';
    for (const key of allowedAttrs) {{
      const val = el.getAttribute ? el.getAttribute(key) : null;
      if (val) {{
        const clean = String(val).replace(/\s+/g, ' ').trim().slice(0, 120);
        if (clean) attrs += ' ' + key + '="' + clean.replace(/"/g, '&quot;') + '"';
      }}
    }}
    let text = '';
    for (const child of el.childNodes || []) {{
      if (child.nodeType === 3) {{
        const part = String(child.nodeValue || '').replace(/\s+/g, ' ').trim();
        if (part) text += (text ? ' ' : '') + part;
      }}
    }}
    if (text.length > 160) {{
      text = text.slice(0, 160);
      truncated = true;
    }}
    const tag = String(el.tagName || 'node').toLowerCase();
    const suffix = text ? ' ' + JSON.stringify(text) : '';
    if (!pushLine(indent + '<' + tag + attrs + '>' + suffix)) return;
    const children = el.children || [];
    if (children.length > 0 && depth >= maxDepth) {{
      truncated = true;
      return;
    }}
    for (const child of children) {{
      walk(child, depth + 1);
      if (truncated) return;
    }}
  }};
  walk(root, 0);
  return JSON.stringify({{ data: lines.join('\n'), truncated }});
}})()"#
    )
}

/// Parses and re-clamps the JSON reply returned by the isolated snapshot script.
pub(crate) fn parse_reply(raw: &str, max_bytes: usize) -> Result<DiagnosticSnapshot, ToolError> {
    if raw.is_empty() {
        return Err(ToolError::new(
            ToolErrorCode::Unavailable,
            "could not capture page snapshot",
        ));
    }
    let reply: Value = serde_json::from_str(raw)
        .map_err(|_| ToolError::new(ToolErrorCode::Unavailable, "invalid page snapshot payload"))?;
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
        return match error {
            "invalid_selector" => Err(ToolError::new(
                ToolErrorCode::InvalidParams,
                "invalid snapshot selector",
            )),
            "selector_not_found" => Err(ToolError::new(
                ToolErrorCode::InvalidParams,
                "snapshot selector matched no element",
            )),
            _ => Err(ToolError::new(
                ToolErrorCode::Unavailable,
                "page snapshot failed",
            )),
        };
    }
    let data = reply
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::new(ToolErrorCode::Unavailable, "missing page snapshot data"))?;
    let js_truncated = reply
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (bounded, rust_truncated) = truncate_utf8(data, max_bytes);
    Ok(DiagnosticSnapshot {
        data: bounded,
        truncated: js_truncated || rust_truncated,
    })
}

fn truncate_utf8(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_owned(), false);
    }
    let end = text.floor_char_boundary(max_bytes);
    (text[..end].to_owned(), true)
}

/// Cancellable isolated-world `WebKitGTK` snapshot evaluation on the GTK main thread.
#[cfg(target_os = "linux")]
pub(crate) struct SnapshotEval {
    cancel: crate::gtk::ffi::Object,
}

#[cfg(target_os = "linux")]
impl SnapshotEval {
    /// Starts an isolated-world script evaluation on `view`, invoking `done` once on the GTK main
    /// context.
    pub(crate) fn start(
        view: crate::gtk::ffi::Object,
        request: &SnapshotRequest,
        done: Box<dyn FnOnce(Result<DiagnosticSnapshot, ToolError>) + 'static>,
    ) -> Result<Self, ToolError> {
        let script = build_script(request)?;
        let script = CString::new(script).map_err(|_| {
            ToolError::new(ToolErrorCode::InvalidParams, "snapshot script contains NUL")
        })?;
        let max_bytes = request.max_bytes();
        // SAFETY: Allocates an owned GCancellable on the GTK main thread.
        let cancel = unsafe { g_cancellable_new() };
        let payload = Box::into_raw(Box::new(EvalCallback { max_bytes, done })).cast();
        // SAFETY: WebKit copies the script and invokes `snapshot_completed` once on the GTK main
        // context; `payload` owns the one-shot callback and is reclaimed in `snapshot_completed`.
        unsafe {
            webkit_web_view_evaluate_javascript(
                view,
                script.as_ptr(),
                -1,
                c"dev.quickie.mcp.snapshot".as_ptr(),
                ptr::null(),
                cancel,
                snapshot_completed,
                payload,
            );
        }
        Ok(Self { cancel })
    }
}

#[cfg(target_os = "linux")]
impl Drop for SnapshotEval {
    fn drop(&mut self) {
        // SAFETY: The owned GCancellable is valid until unreffed below.
        unsafe { g_cancellable_cancel(self.cancel) };
        // SAFETY: Releases the single reference created in `SnapshotEval::start`.
        unsafe { crate::gtk::ffi::g_object_unref(self.cancel) };
    }
}

#[cfg(target_os = "linux")]
struct EvalCallback {
    max_bytes: usize,
    done: Box<dyn FnOnce(Result<DiagnosticSnapshot, ToolError>) + 'static>,
}

#[cfg(target_os = "linux")]
extern "C" fn snapshot_completed(
    view: crate::gtk::ffi::Object,
    result: crate::gtk::ffi::Object,
    data: crate::gtk::ffi::Object,
) {
    // SAFETY: Reclaims the unique boxed callback created in `SnapshotEval::start`.
    let callback = unsafe { Box::from_raw(data.cast::<EvalCallback>()) };
    // SAFETY: Both arguments are live for this WebKit finish callback; omitting GError suppresses
    // raw script exception details from leaking across the MCP boundary.
    let value =
        unsafe { webkit_web_view_evaluate_javascript_finish(view, result, ptr::null_mut()) };
    if value.is_null() {
        (callback.done)(Err(ToolError::new(
            ToolErrorCode::Unavailable,
            "could not capture page snapshot",
        )));
        return;
    }
    // SAFETY: Transfers ownership of the JSCValue string allocation from WebKit.
    let text = unsafe { jsc_value_to_string(value) };
    // SAFETY: Releases the owned JSCValue returned by `evaluate_javascript_finish`.
    unsafe { crate::gtk::ffi::g_object_unref(value) };
    let outcome = if text.is_null() {
        Err(ToolError::new(
            ToolErrorCode::Unavailable,
            "could not read page snapshot",
        ))
    } else {
        // SAFETY: The non-null GLib string is NUL-terminated and copied before freeing.
        let raw = unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: `jsc_value_to_string` allocates with GLib's allocator.
        unsafe { g_free(text.cast()) };
        parse_reply(&raw, callback.max_bytes)
    };
    (callback.done)(outcome);
}

#[cfg(target_os = "linux")]
#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_web_view_evaluate_javascript(
        view: crate::gtk::ffi::Object,
        script: *const c_char,
        length: isize,
        world: *const c_char,
        source_uri: *const c_char,
        cancel: crate::gtk::ffi::Object,
        callback: extern "C" fn(
            crate::gtk::ffi::Object,
            crate::gtk::ffi::Object,
            crate::gtk::ffi::Object,
        ),
        data: crate::gtk::ffi::Object,
    );
    fn webkit_web_view_evaluate_javascript_finish(
        view: crate::gtk::ffi::Object,
        result: crate::gtk::ffi::Object,
        error: *mut *mut crate::gtk::ffi::GError,
    ) -> crate::gtk::ffi::Object;
}

#[cfg(target_os = "linux")]
#[link(name = "javascriptcoregtk-6.0")]
unsafe extern "C" {
    fn jsc_value_to_string(value: crate::gtk::ffi::Object) -> *mut c_char;
}

#[cfg(target_os = "linux")]
#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_free(memory: crate::gtk::ffi::Object);
}

#[cfg(target_os = "linux")]
#[link(name = "gio-2.0")]
unsafe extern "C" {
    fn g_cancellable_new() -> crate::gtk::ffi::Object;
    fn g_cancellable_cancel(cancel: crate::gtk::ffi::Object);
}
