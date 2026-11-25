//! Bounded isolated-world DOM and visible-text snapshots for read-only diagnostics.
//!
//! Runs fixed scripts in Quickie's isolated `WKContentWorld` using the existing `objc2-web-kit`
//! evaluation path, enforces node/depth/byte bounds in both JS and Rust, and refuses
//! [`SnapshotKind::Accessibility`] until native accessibility data is consumed.

use objc2::MainThreadMarker;
use objc2_web_kit::WKWebView;
use quickie_application::mcp::{SnapshotKind, SnapshotRequest};
use quickie_application::{DiagnosticSnapshot, EngineError};
use serde_json::Value;

use crate::page::scripts;

/// Captures a bounded read-only DOM or visible-text snapshot from `view`'s main frame in
/// Quickie's isolated content world.
///
/// # Errors
///
/// Returns [`EngineError`] if [`SnapshotKind::Accessibility`] is requested (native accessibility
/// snapshots are not yet wired).
pub(crate) fn capture(
    view: &WKWebView,
    request: &SnapshotRequest,
    mtm: MainThreadMarker,
    done: Box<dyn FnOnce(Result<DiagnosticSnapshot, EngineError>) + 'static>,
) -> Result<(), EngineError> {
    let script = build_script(request)?;
    let max_bytes = request.max_bytes();
    let world = scripts::world(mtm);
    scripts::evaluate(view, &world, None, &script, move |answer| {
        done(parse_reply(&answer, max_bytes));
    });
    Ok(())
}

pub(crate) fn build_script(request: &SnapshotRequest) -> Result<String, EngineError> {
    if request.kind() == SnapshotKind::Accessibility {
        return Err(EngineError(
            "native accessibility snapshots are unavailable".to_owned(),
        ));
    }
    let selector_json = match request.selector() {
        Some(selector) => serde_json::to_string(selector)
            .map_err(|error| EngineError(format!("invalid snapshot selector: {error}")))?,
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
        SnapshotKind::Accessibility => Err(EngineError(
            "native accessibility snapshots are unavailable".to_owned(),
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

pub(crate) fn parse_reply(raw: &str, max_bytes: usize) -> Result<DiagnosticSnapshot, EngineError> {
    if raw.is_empty() {
        return Err(EngineError("could not capture page snapshot".to_owned()));
    }
    let reply: Value = serde_json::from_str(raw)
        .map_err(|_| EngineError("invalid page snapshot payload".to_owned()))?;
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
        let message = match error {
            "invalid_selector" => "invalid snapshot selector",
            "selector_not_found" => "snapshot selector matched no element",
            _ => "page snapshot failed",
        };
        return Err(EngineError(message.to_owned()));
    }
    let data = reply
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError("missing page snapshot data".to_owned()))?;
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
