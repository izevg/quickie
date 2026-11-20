//! The link-hint scripts, and their answers read: where the targets are on screen and what
//! following one asks of the browser. Pure, so the arithmetic is tested without `WebKit`.

use objc2_foundation::NSSize;
use quickie_application::{Followed, LinkTarget, PageRect};
use url::Url;

/// Finds the links and controls the user can see, in document order, and keeps them in the
/// world's own global for [`follow_script`]. Answers `innerWidth,innerHeight` and then
/// `left,top,width,height,link` per target, in CSS pixels of the viewport, `;`-separated.
pub(crate) const COLLECT_SCRIPT: &str = r#"(() => {
  const W = innerWidth, H = innerHeight;
  const sel = 'a[href],area[href],button,input:not([type=hidden]),select,textarea,summary,' +
    'label[for],[role=button],[role=link],[role=checkbox],[role=tab],[role=menuitem],' +
    '[onclick],[contenteditable=""],[contenteditable=true],[tabindex]:not([tabindex="-1"])';
  const kept = [], seen = new Set(), out = [W + ',' + H];
  for (const el of document.querySelectorAll(sel)) {
    if (kept.length >= 500) break;
    if (el.disabled) continue;
    const outer = el.parentElement && el.parentElement.closest(sel);
    if (outer && seen.has(outer)) continue;
    const r = Array.from(el.getClientRects()).find((r) => r.width >= 2 && r.height >= 2 &&
      r.bottom > 0 && r.right > 0 && r.top < H && r.left < W);
    if (!r) continue;
    const x = Math.min(Math.max(r.left + r.width / 2, 0), W - 1);
    const y = Math.min(Math.max(r.top + r.height / 2, 0), H - 1);
    const hit = document.elementFromPoint(x, y);
    if (!hit || !(el.contains(hit) || hit.contains(el))) continue;
    kept.push(el);
    seen.add(el);
    const link = (el.tagName === 'A' || el.tagName === 'AREA') && el.href ? 1 : 0;
    out.push([r.left, r.top, r.width, r.height].map(Math.round).join(',') + ',' + link);
  }
  window.__quickieHints = kept;
  return out.join(';');
})()"#;

/// Follows target `index` of the last [`COLLECT_SCRIPT`]: `bg:` and the URL of a link to open
/// in a background tab when `background`, `new:` and the URL of a link that asks for a new
/// window, otherwise it focuses the element, clicks it unless it takes text, and answers `""`.
pub(crate) fn follow_script(index: usize, background: bool) -> String {
    format!(
        r"(() => {{
  const el = (window.__quickieHints || [])[{index}];
  window.__quickieHints = [];
  if (!el || !el.isConnected) return '';
  const href = (el.tagName === 'A' || el.tagName === 'AREA') && el.href ? el.href : '';
  if (href && {background}) return 'bg:' + href;
  if (href && el.target === '_blank') return 'new:' + href;
  const textless = new Set(['button', 'checkbox', 'color', 'file', 'image', 'radio', 'range',
    'reset', 'submit']);
  const text = el.isContentEditable || el.tagName === 'TEXTAREA' ||
    (el.tagName === 'INPUT' && !textless.has(String(el.type).toLowerCase()));
  el.focus({{ preventScroll: true }});
  if (!text) el.click();
  return '';
}})()"
    )
}

/// The targets in `answer`, as the collect script wrote them, placed in a view of `view`
/// size, the origin at its top left. The page's viewport fills the view's width and ends at
/// its bottom edge; a viewport shorter than the view sits under a top inset. A malformed entry
/// is skipped.
pub(crate) fn targets(answer: &str, view: NSSize) -> Vec<LinkTarget> {
    let mut entries = answer.split(';');
    let Some([css_width, css_height]) = entries.next().and_then(numbers::<2>) else {
        return Vec::new();
    };
    let scale = if css_width > 0.0 {
        view.width / css_width
    } else {
        1.0
    };
    // The viewport's top edge, in the view.
    let inset = view.height - css_height * scale;
    entries
        .filter_map(numbers::<5>)
        .map(|[left, top, width, height, link]| LinkTarget {
            frame: PageRect {
                x: left * scale,
                y: inset + top * scale,
                width: width * scale,
                height: height * scale,
            },
            link: link > 0.0,
        })
        .collect()
}

/// What the follow script's `answer` asks for.
pub(crate) fn followed(answer: &str) -> Followed {
    let url = |rest: &str| Url::parse(rest).ok();
    if let Some(url) = answer.strip_prefix("bg:").and_then(url) {
        return Followed::Background(url);
    }
    if let Some(url) = answer.strip_prefix("new:").and_then(url) {
        return Followed::NewTab(url);
    }
    Followed::Done
}

/// Exactly `N` comma-separated finite numbers.
fn numbers<const N: usize>(entry: &str) -> Option<[f64; N]> {
    let mut parsed = [0.0; N];
    let mut parts = entry.split(',');
    for slot in &mut parsed {
        *slot = parts
            .next()?
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())?;
    }
    parts.next().is_none().then_some(parsed)
}

#[cfg(test)]
mod tests {
    use objc2_foundation::NSSize;
    use quickie_application::{Followed, LinkTarget, PageRect};
    use url::Url;

    use super::{follow_script, followed, targets};

    #[test]
    fn the_follow_script_names_its_target_and_mode() {
        let script = follow_script(17, true);

        assert!(script.contains("[17]"));
        assert!(script.contains("if (href && true)"));
        assert!(follow_script(0, false).contains("if (href && false)"));
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> PageRect {
        PageRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn targets_keep_their_place_from_the_view_top_left() {
        let found = targets(
            "800,600;10,20,100,30,1;0,570,50,30,0",
            NSSize::new(800.0, 600.0),
        );

        assert_eq!(
            found,
            [
                LinkTarget {
                    frame: rect(10.0, 20.0, 100.0, 30.0),
                    link: true,
                },
                LinkTarget {
                    frame: rect(0.0, 570.0, 50.0, 30.0),
                    link: false,
                },
            ]
        );
    }

    #[test]
    fn a_top_inset_leaves_the_bottom_aligned() {
        // The view is 28 points taller than the viewport: a title bar inset.
        let found = targets("800,572;0,0,10,10,0", NSSize::new(800.0, 600.0));

        assert_eq!(found[0].frame, rect(0.0, 28.0, 10.0, 10.0));
    }

    #[test]
    fn a_zoomed_page_scales_into_view_points() {
        let found = targets("400,300;10,10,20,20,1", NSSize::new(800.0, 600.0));

        assert_eq!(found[0].frame, rect(20.0, 20.0, 40.0, 40.0));
    }

    #[test]
    fn malformed_answers_and_entries_are_skipped() {
        let view = NSSize::new(100.0, 100.0);

        assert_eq!(targets("", view), []);
        assert_eq!(targets("100", view), []);
        assert_eq!(targets("100,100,1", view), []);
        assert_eq!(
            targets("100,100;1,2,3;x,1,1,1,1;1,1,1,1,1,1;NaN,1,1,1,1", view),
            []
        );
        assert_eq!(targets("100,100;1,2,3,4,0", view).len(), 1);
    }

    #[test]
    fn a_zero_wide_viewport_does_not_scale() {
        let found = targets("0,100;1,2,3,4,0", NSSize::new(50.0, 100.0));

        assert_eq!(found[0].frame, rect(1.0, 2.0, 3.0, 4.0));
    }

    #[test]
    fn following_answers_what_is_left_to_do() {
        let url = Url::parse("https://example.com/a").unwrap();

        assert_eq!(
            followed("bg:https://example.com/a"),
            Followed::Background(url.clone())
        );
        assert_eq!(followed("new:https://example.com/a"), Followed::NewTab(url));
        assert_eq!(followed(""), Followed::Done);
        assert_eq!(followed("bg:not a url"), Followed::Done);
        assert_eq!(followed("https://example.com/"), Followed::Done);
    }
}
