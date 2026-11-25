//! Fixed, application-owned isolated-world scripts for Agent element projection, waits, and
//! semantic DOM accessibility fallback.
//!
//! No arbitrary JavaScript from MCP callers is ever accepted. All parameters are JSON-encoded
//! literals and element references are bound to a per-context, per-document-epoch token inside
//! the isolated world `dev.quickie.mcp.agent`.

use quickie_application::mcp::{AgentContextId, SnapshotRequest};
use quickie_mcp::browser::{
    AgentElementRef, AgentLocateResult, AgentLocator, AgentLocatorKind, AgentObservedState,
    AgentWaitCondition, AgentWaitStage, ElementBounds, ElementProvenance, FrameScope,
    LocatedElement, MAX_ELEMENT_ID_BYTES, MAX_LOCATE_RESULTS, ToolError, ToolErrorCode,
};
use serde_json::Value;

/// Shared isolated-world JavaScript helper defining semantic role/name/visibility computation.
const SEMANTIC_HELPERS_JS: &str = r#"
  const inferRole = (el) => {
    const explicit = el.getAttribute && el.getAttribute('role');
    if (explicit) return String(explicit).trim().toLowerCase().split(/\s+/)[0] || '';
    const tag = String(el.tagName || '').toLowerCase();
    if (tag === 'button') return 'button';
    if (tag === 'a' && el.hasAttribute('href')) return 'link';
    if (tag === 'textarea') return 'textbox';
    if (tag === 'select') return el.multiple ? 'listbox' : 'combobox';
    if (tag === 'img') return 'img';
    if (tag === 'nav') return 'navigation';
    if (tag === 'main') return 'main';
    if (tag === 'form') return 'form';
    if (tag === 'table') return 'table';
    if (tag === 'ul' || tag === 'ol') return 'list';
    if (tag === 'li') return 'listitem';
    if (/^h[1-6]$/.test(tag)) return 'heading';
    if (tag === 'input') {
      const t = String(el.getAttribute('type') || 'text').toLowerCase();
      if (t === 'button' || t === 'submit' || t === 'reset') return 'button';
      if (t === 'checkbox') return 'checkbox';
      if (t === 'radio') return 'radio';
      if (t === 'search') return 'searchbox';
      if (t === 'number') return 'spinbutton';
      if (t === 'range') return 'slider';
      return 'textbox';
    }
    return 'generic';
  };
  const computeName = (el) => {
    const aria = el.getAttribute && el.getAttribute('aria-label');
    if (aria && aria.trim()) return aria.trim().slice(0, 200);
    const labelledBy = el.getAttribute && el.getAttribute('aria-labelledby');
    if (labelledBy) {
      const parts = labelledBy.trim().split(/\s+/).map((id) => {
        const ref = document.getElementById(id);
        return ref ? String(ref.innerText || ref.textContent || '').trim() : '';
      }).filter(Boolean);
      if (parts.length) return parts.join(' ').slice(0, 200);
    }
    if (el.id) {
      try {
        const lbl = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
        if (lbl) {
          const txt = String(lbl.innerText || lbl.textContent || '').trim();
          if (txt) return txt.slice(0, 200);
        }
      } catch (_) {}
    }
    const alt = el.getAttribute && el.getAttribute('alt');
    if (alt && alt.trim()) return alt.trim().slice(0, 200);
    const title = el.getAttribute && el.getAttribute('title');
    if (title && title.trim()) return title.trim().slice(0, 200);
    const placeholder = el.getAttribute && el.getAttribute('placeholder');
    if (placeholder && placeholder.trim()) return placeholder.trim().slice(0, 200);
    const txt = String(el.innerText || el.textContent || '').replace(/\s+/g, ' ').trim();
    return txt ? txt.slice(0, 200) : '';
  };
  const isVisible = (el) => {
    if (!el || !el.isConnected) return false;
    const style = window.getComputedStyle(el);
    if (!style || style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') {
      return false;
    }
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  const registryFor = (token) => {
    const root = globalThis.__quickieAgentRefs || (globalThis.__quickieAgentRefs = Object.create(null));
    if (root.token !== token) {
      root.token = token;
      root.nextId = 1;
      root.byId = new Map();
      root.byEl = new WeakMap();
    }
    return root;
  };
  const assignId = (reg, token, el) => {
    let id = reg.byEl.get(el);
    if (!id) {
      id = 'el-' + token + '-' + (reg.nextId++);
      reg.byEl.set(el, id);
      reg.byId.set(id, el);
    }
    return id;
  };
"#;

/// Resolved live element geometry and approval classification returned before `agent_act`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedElement {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) role: String,
    pub(crate) tag: String,
    pub(crate) requires_approval: bool,
    pub(crate) approval_reason: Option<String>,
    pub(crate) is_editable: bool,
}

/// Builds a fixed isolated-world script to locate elements by CSS, text, or semantic role/name.
pub(crate) fn build_locate_script(
    locator: &AgentLocator,
    max_results: usize,
    doc_token: &str,
) -> Result<String, ToolError> {
    let limit = max_results.clamp(1, MAX_LOCATE_RESULTS);
    let token_json = to_json_literal(doc_token)?;
    let matcher_js = match locator {
        AgentLocator::Css { selector } => {
            let selector_json = to_json_literal(selector)?;
            format!(
                r"let candidates = [];
  try {{ candidates = Array.from(document.querySelectorAll({selector_json})); }}
  catch (_) {{ return JSON.stringify({{ error: 'invalid_selector' }}); }}"
            )
        }
        AgentLocator::Text { text, exact } => {
            let text_json = to_json_literal(text)?;
            format!(
                r"const needle = {text_json};
  const exact = {exact};
  const norm = needle.replace(/\s+/g, ' ').trim();
  const candidates = Array.from(document.querySelectorAll('*')).filter((el) => {{
    if (!isVisible(el)) return false;
    const own = String(el.innerText || '').replace(/\s+/g, ' ').trim();
    if (!own) return false;
    const matched = exact ? (own === norm) : own.includes(norm);
    if (!matched) return false;
    for (const child of el.children || []) {{
      if (isVisible(child)) {{
        const cText = String(child.innerText || '').replace(/\s+/g, ' ').trim();
        if (exact ? (cText === norm) : cText.includes(norm)) return false;
      }}
    }}
    return true;
  }});"
            )
        }
        AgentLocator::Role { role, name, exact } => {
            let role_json = to_json_literal(role)?;
            let name_json = match name.as_deref() {
                Some(n) => to_json_literal(n)?,
                None => "null".to_owned(),
            };
            format!(
                r"const targetRole = {role_json};
  const targetName = {name_json};
  const exact = {exact};
  const candidates = Array.from(document.querySelectorAll('*')).filter((el) => {{
    if (!isVisible(el)) return false;
    if (inferRole(el) !== targetRole) return false;
    if (targetName === null) return true;
    const label = computeName(el);
    return exact ? (label === targetName) : label.toLowerCase().includes(targetName.toLowerCase());
  }});"
            )
        }
    };

    Ok(format!(
        r"(() => {{
  {SEMANTIC_HELPERS_JS}
  const token = {token_json};
  const limit = {limit};
  {matcher_js}
  const reg = registryFor(token);
  const out = [];
  let truncated = false;
  for (const el of candidates) {{
    if (!isVisible(el)) continue;
    if (out.length >= limit) {{
      truncated = true;
      break;
    }}
    const rect = el.getBoundingClientRect();
    const id = assignId(reg, token, el);
    const name = computeName(el);
    out.push({{
      element_id: id,
      role: inferRole(el) || 'generic',
      name: name || null,
      tag: String(el.tagName || '').toLowerCase() || null,
      bounds: {{ x: rect.x, y: rect.y, width: rect.width, height: rect.height }}
    }});
  }}
  return JSON.stringify({{ elements: out, truncated }});
}})()"
    ))
}

/// Parses the JSON reply from [`build_locate_script`] into a validated [`AgentLocateResult`].
pub(crate) fn parse_locate_reply(
    raw: &str,
    context_id: AgentContextId,
    epoch: u64,
    origin: &str,
    locator_kind: AgentLocatorKind,
    max_results: usize,
) -> Result<AgentLocateResult, ToolError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| ToolError::new(ToolErrorCode::Unavailable, "invalid locate payload"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return match error {
            "invalid_selector" => Err(ToolError::new(
                ToolErrorCode::InvalidParams,
                "invalid CSS selector",
            )),
            _ => Err(ToolError::new(
                ToolErrorCode::Unavailable,
                "element locate failed",
            )),
        };
    }
    let items = value
        .get("elements")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::new(ToolErrorCode::Unavailable, "missing locate elements"))?;
    let limit = max_results.clamp(1, MAX_LOCATE_RESULTS);
    let js_truncated = value
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let truncated = js_truncated || items.len() > limit;

    let mut elements = Vec::with_capacity(items.len().min(limit));
    for item in items.iter().take(limit) {
        let Some(element_id) = item
            .get("element_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= MAX_ELEMENT_ID_BYTES)
        else {
            continue;
        };
        let role = item
            .get("role")
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty())
            .unwrap_or("generic")
            .to_owned();
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .map(str::to_owned);
        let tag = item
            .get("tag")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_owned);
        let bounds = item.get("bounds").and_then(parse_bounds);
        elements.push(LocatedElement {
            element_ref: AgentElementRef {
                context_id,
                epoch,
                element_id: element_id.to_owned(),
            },
            role,
            name,
            tag,
            bounds,
            provenance: ElementProvenance {
                origin: origin.to_owned(),
                frame: FrameScope::MainFrame,
                locator_kind,
            },
        });
    }

    Ok(AgentLocateResult {
        context_id,
        epoch,
        elements,
        truncated,
    })
}

/// Builds a fixed isolated-world script to resolve a document-bound element reference, compute its
/// viewport center, and classify whether interacting with it is irreversible or cross-origin.
pub(crate) fn build_resolve_element_script(
    element_id: &str,
    doc_token: &str,
    scroll_into_view: bool,
) -> Result<String, ToolError> {
    let element_id_json = to_json_literal(element_id)?;
    let token_json = to_json_literal(doc_token)?;
    Ok(format!(
        r"(() => {{
  {SEMANTIC_HELPERS_JS}
  const reg = registryFor({token_json});
  const el = reg.byId.get({element_id_json});
  if (!el || !el.isConnected) {{
    return JSON.stringify({{ error: 'stale_element' }});
  }}
  if ({scroll_into_view} && typeof el.scrollIntoView === 'function') {{
    el.scrollIntoView({{ block: 'center', inline: 'center', behavior: 'instant' }});
  }}
  const rect = el.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) {{
    return JSON.stringify({{ error: 'not_visible' }});
  }}
  const tag = String(el.tagName || '').toLowerCase();
  const type = String(el.getAttribute && el.getAttribute('type') || '').toLowerCase();
  const role = inferRole(el) || 'generic';
  let requiresApproval = false;
  let approvalReason = null;
  if (tag === 'input' && (type === 'submit' || type === 'password' || type === 'file')) {{
    requiresApproval = true;
    approvalReason = 'sensitive or form-submitting input requires explicit approval';
  }} else if (tag === 'button' && (!type || type === 'submit') && el.closest && el.closest('form')) {{
    requiresApproval = true;
    approvalReason = 'form submission button requires explicit approval';
  }} else if (tag === 'a' && el.href) {{
    try {{
      const dest = new URL(el.href, window.location.href);
      if (dest.origin !== window.location.origin) {{
        requiresApproval = true;
        approvalReason = 'cross-origin link activation requires explicit approval';
      }}
    }} catch (_) {{}}
  }}
  const isEditable = tag === 'textarea' || (tag === 'input' && !['button','submit','reset','checkbox','radio','file'].includes(type)) || Boolean(el.isContentEditable);
  return JSON.stringify({{
    x: rect.x + rect.width / 2,
    y: rect.y + rect.height / 2,
    role,
    tag,
    requires_approval: requiresApproval,
    approval_reason: approvalReason,
    is_editable: isEditable
  }});
}})()"
    ))
}

/// Parses the JSON reply from [`build_resolve_element_script`].
pub(crate) fn parse_resolve_element_reply(raw: &str) -> Result<ResolvedElement, ToolError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| ToolError::new(ToolErrorCode::Unavailable, "invalid element lookup reply"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return match error {
            "stale_element" => Err(ToolError::new(
                ToolErrorCode::StaleDocument,
                "element reference is stale or detached",
            )),
            "not_visible" => Err(ToolError::new(
                ToolErrorCode::InvalidParams,
                "target element is not visible in the viewport",
            )),
            _ => Err(ToolError::new(
                ToolErrorCode::Unavailable,
                "could not resolve target element",
            )),
        };
    }
    let x = value
        .get("x")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::InvalidParams,
                "target element is outside the viewport",
            )
        })?;
    let y = value
        .get("y")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::InvalidParams,
                "target element is outside the viewport",
            )
        })?;
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("generic")
        .to_owned();
    let tag = value
        .get("tag")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let requires_approval = value
        .get("requires_approval")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let approval_reason = value
        .get("approval_reason")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let is_editable = value
        .get("is_editable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(ResolvedElement {
        x,
        y,
        role,
        tag,
        requires_approval,
        approval_reason,
        is_editable,
    })
}

/// Builds a fixed isolated-world script to read bounded post-action state (never raw input text).
pub(crate) fn build_observed_state_script(
    element_id: Option<&str>,
    doc_token: &str,
) -> Result<String, ToolError> {
    let element_id_json = match element_id {
        Some(id) => to_json_literal(id)?,
        None => "null".to_owned(),
    };
    let token_json = to_json_literal(doc_token)?;
    Ok(format!(
        r"(() => {{
  {SEMANTIC_HELPERS_JS}
  const active = document.activeElement;
  const focusedRole = active && active !== document.body ? (inferRole(active) || 'generic') : null;
  let target = active;
  const eid = {element_id_json};
  if (eid !== null) {{
    const reg = registryFor({token_json});
    const resolved = reg.byId.get(eid);
    if (resolved && resolved.isConnected) target = resolved;
  }}
  let valueLength = null;
  if (target && typeof target.value === 'string') {{
    valueLength = target.value.length;
  }}
  return JSON.stringify({{
    focused_role: focusedRole,
    value_length: valueLength,
    scroll_x: Math.round(window.scrollX || 0),
    scroll_y: Math.round(window.scrollY || 0)
  }});
}})()"
    ))
}

/// Parses the JSON reply from [`build_observed_state_script`].
#[must_use]
pub(crate) fn parse_observed_state_reply(raw: &str, navigated: bool) -> AgentObservedState {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return AgentObservedState {
            navigated,
            ..AgentObservedState::default()
        };
    };
    AgentObservedState {
        navigated,
        focused_role: value
            .get("focused_role")
            .and_then(Value::as_str)
            .map(str::to_owned),
        value_length: value
            .get("value_length")
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok()),
        scroll_x: value
            .get("scroll_x")
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok()),
        scroll_y: value
            .get("scroll_y")
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok()),
    }
}

/// Builds a fixed isolated-world script to evaluate an [`AgentWaitCondition`] once.
pub(crate) fn build_wait_check_script(condition: &AgentWaitCondition) -> Result<String, ToolError> {
    match condition {
        AgentWaitCondition::LoadStage { stage } => {
            let check = match stage {
                AgentWaitStage::DomContentLoaded => {
                    "document.readyState === 'interactive' || document.readyState === 'complete'"
                }
                AgentWaitStage::Load => "document.readyState === 'complete'",
                AgentWaitStage::None => {
                    return Err(ToolError::new(
                        ToolErrorCode::InvalidParams,
                        "load_stage wait requires dom_content_loaded or load",
                    ));
                }
            };
            Ok(format!(r"(() => JSON.stringify({{ matched: {check} }}))()"))
        }
        AgentWaitCondition::TextPresent { text, exact } => {
            let text_json = to_json_literal(text)?;
            Ok(format!(
                r"(() => {{
  const needle = {text_json}.replace(/\s+/g, ' ').trim();
  const root = document.body || document.documentElement;
  const hay = root ? String(root.innerText || '').replace(/\s+/g, ' ').trim() : '';
  const matched = {exact} ? (hay === needle) : hay.includes(needle);
  return JSON.stringify({{ matched }});
}})()"
            ))
        }
        AgentWaitCondition::LocatorVisible { locator }
        | AgentWaitCondition::LocatorHidden { locator } => {
            let want_visible = matches!(condition, AgentWaitCondition::LocatorVisible { .. });
            let locate_script = build_locate_script(locator, 1, "wait-probe")?;
            Ok(format!(
                r"(() => {{
  const res = JSON.parse({locate_script});
  if (res.error) return JSON.stringify({{ error: res.error }});
  const anyVisible = Array.isArray(res.elements) && res.elements.length > 0;
  return JSON.stringify({{ matched: {want_visible} ? anyVisible : !anyVisible }});
}})()"
            ))
        }
    }
}

/// Parses the JSON reply from [`build_wait_check_script`].
pub(crate) fn parse_wait_check_reply(raw: &str) -> Result<bool, ToolError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| ToolError::new(ToolErrorCode::Unavailable, "invalid wait probe payload"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return match error {
            "invalid_selector" => Err(ToolError::new(
                ToolErrorCode::InvalidParams,
                "invalid CSS selector",
            )),
            _ => Err(ToolError::new(
                ToolErrorCode::Unavailable,
                "wait condition check failed",
            )),
        };
    }
    value
        .get("matched")
        .and_then(Value::as_bool)
        .ok_or_else(|| ToolError::new(ToolErrorCode::Unavailable, "missing wait match flag"))
}

/// Builds a bounded semantic DOM accessibility fallback script for isolated Agent contexts.
///
/// Output format matches `crate::mcp::snapshot::parse_reply` (`{ data, truncated }`). Callers MUST
/// report [`quickie_mcp::AccessibilitySource::SemanticDomFallback`].
pub(crate) fn build_accessibility_fallback_script(
    request: &SnapshotRequest,
) -> Result<String, ToolError> {
    let selector_json = match request.selector() {
        Some(selector) => to_json_literal(selector)?,
        None => "null".to_owned(),
    };
    let max_nodes = request.max_nodes();
    let max_depth = request.max_depth();
    let max_bytes = request.max_bytes();
    Ok(format!(
        r"(() => {{
  {SEMANTIC_HELPERS_JS}
  const selector = {selector_json};
  const maxNodes = {max_nodes};
  const maxDepth = {max_depth};
  const maxBytes = {max_bytes};
  let root = null;
  if (selector !== null) {{
    try {{ root = document.querySelector(selector); }}
    catch (_) {{ return JSON.stringify({{ error: 'invalid_selector' }}); }}
    if (!root) return JSON.stringify({{ error: 'selector_not_found' }});
  }} else {{
    root = document.body || document.documentElement;
  }}
  if (!root) return JSON.stringify({{ data: '', truncated: false }});
  const enc = new TextEncoder();
  let nodes = 0;
  let bytes = 0;
  let truncated = false;
  const lines = [];
  const pushLine = (line) => {{
    const len = enc.encode(line).length + (lines.length ? 1 : 0);
    if (bytes + len > maxBytes) {{ truncated = true; return false; }}
    lines.push(line);
    bytes += len;
    return true;
  }};
  const walk = (el, depth) => {{
    if (truncated || !isVisible(el)) return;
    if (nodes >= maxNodes) {{ truncated = true; return; }}
    nodes += 1;
    const role = inferRole(el) || 'generic';
    const name = computeName(el);
    const indent = '  '.repeat(depth);
    const label = name ? ' ' + JSON.stringify(name) : '';
    if (!pushLine(indent + role + label)) return;
    const children = el.children || [];
    if (children.length > 0 && depth >= maxDepth) {{ truncated = true; return; }}
    for (const child of children) {{
      walk(child, depth + 1);
      if (truncated) return;
    }}
  }};
  walk(root, 0);
  return JSON.stringify({{ data: lines.join('\n'), truncated }});
}})()"
    ))
}

fn parse_bounds(value: &Value) -> Option<ElementBounds> {
    let bounds = ElementBounds {
        x: value.get("x")?.as_f64()?,
        y: value.get("y")?.as_f64()?,
        width: value.get("width")?.as_f64()?,
        height: value.get("height")?.as_f64()?,
    };
    bounds.is_valid().then_some(bounds)
}

fn to_json_literal(raw: &str) -> Result<String, ToolError> {
    serde_json::to_string(raw)
        .map_err(|_| ToolError::new(ToolErrorCode::InvalidParams, "invalid string parameter"))
}
