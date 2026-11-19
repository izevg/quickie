//! One enabled filter list as a row (`docs/design/inventory.md`, entities `filter_list_row`):
//! its name, and when it was last fetched. It lives in the popover's slice until an `entities`
//! crate exists.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSLineBreakMode, NSTextAlignment, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_domain_shields::{FilterListId, Timestamp};
use quickie_ui_shared::format;
use quickie_ui_shared::tokens::{TextRole, color, space};

/// A row's height.
pub(crate) const ROW_HEIGHT: CGFloat = 24.0;

/// When a list was fetched, as the row says it: `Updated 3 h ago`, `Not fetched yet`.
pub(crate) fn fetched(fetched: Option<Timestamp>, now: Timestamp) -> String {
    match fetched {
        Some(at) => format!("Updated {}", format::ago(now.millis_since(at))),
        None => "Not fetched yet".to_owned(),
    }
}

pub(crate) struct FilterListRow {
    view: Retained<NSView>,
}

impl FilterListRow {
    /// The row of `list`, last fetched at `at`, `width` wide.
    pub(crate) fn new(
        list: FilterListId,
        at: Option<Timestamp>,
        now: Timestamp,
        width: CGFloat,
        mtm: MainThreadMarker,
    ) -> Self {
        let view = NSView::new(mtm);
        view.setFrameSize(NSSize::new(width, ROW_HEIGHT));
        let title = text(list.title(), TextRole::RowTitle, mtm);
        let detail = text(&fetched(at, now), TextRole::RowDetail, mtm);
        detail.setTextColor(Some(&color::secondary_label()));
        detail.setAlignment(NSTextAlignment::Right);
        let detail_width = detail.frame().size.width.min(width / 2.0);
        let middle = |height: CGFloat| ((ROW_HEIGHT - height) / 2.0).round();
        let title_height = title.frame().size.height;
        title.setFrame(NSRect::new(
            NSPoint::new(0.0, middle(title_height)),
            NSSize::new(width - detail_width - space::SM, title_height),
        ));
        let detail_height = detail.frame().size.height;
        detail.setFrame(NSRect::new(
            NSPoint::new(width - detail_width, middle(detail_height)),
            NSSize::new(detail_width, detail_height),
        ));
        view.addSubview(&title);
        view.addSubview(&detail);
        Self { view }
    }

    pub(crate) fn view(&self) -> &NSView {
        &self.view
    }
}

fn text(value: &str, role: TextRole, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(value), mtm);
    field.setFont(Some(&role.font()));
    field.setTextColor(Some(&color::label()));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field
}

#[cfg(test)]
mod tests {
    use quickie_domain_shields::Timestamp;

    use super::fetched;

    #[test]
    fn a_fetched_list_says_how_long_ago() {
        let now = Timestamp::from_millis(10 * 3_600_000);

        assert_eq!(
            fetched(Some(Timestamp::from_millis(7 * 3_600_000)), now),
            "Updated 3 h ago"
        );
        assert_eq!(fetched(Some(now), now), "Updated just now");
    }

    #[test]
    fn a_list_never_fetched_says_so() {
        assert_eq!(fetched(None, Timestamp::from_millis(0)), "Not fetched yet");
    }
}
