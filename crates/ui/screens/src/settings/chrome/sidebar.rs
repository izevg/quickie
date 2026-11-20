//! The sidebar (`docs/design/mocks/settings/`): a search field under the titlebar's buttons,
//! and the panes as a list on the sidebar material. Typing in the search keeps the panes whose
//! names or settings match, and `⏎` or `↓` goes to the first of them. `↑` `↓` move between
//! panes and show each at once; a click does the same. `VoiceOver` reads each row by its pane.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSBorderType, NSColor, NSControl,
    NSControlTextEditingDelegate, NSImage, NSImageSymbolConfiguration, NSImageView, NSScrollView,
    NSSearchField, NSSearchFieldDelegate, NSTableColumn, NSTableRowView, NSTableView,
    NSTableViewDataSource, NSTableViewDelegate, NSTableViewStyle, NSTextField, NSTextFieldDelegate,
    NSTextView, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSIndexSet, NSInteger, NSNotification, NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, space};

use super::row_view::{RowView, Tone};
use super::{HEIGHT, Page};
use crate::settings::Pane;

/// The sidebar's width.
pub const WIDTH: CGFloat = 220.0;
/// A pane's row.
const ROW_HEIGHT: CGFloat = 32.0;
/// The search field's height.
const SEARCH_HEIGHT: CGFloat = 28.0;

type Choose = Rc<dyn Fn(Pane)>;

pub struct Ivars {
    choose: RefCell<Option<Choose>>,
    /// While the screen selects a row itself, the change is not told back.
    quiet: Cell<bool>,
    /// The panes the search keeps, in sidebar order.
    shown: RefCell<Vec<Pane>>,
    /// The pane on screen, to select again when the search keeps it.
    current: Cell<Option<Pane>>,
    table: RefCell<Option<Retained<NSTableView>>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `SidebarSource` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieSettingsSidebar"]
    #[ivars = Ivars]
    pub struct SidebarSource;

    unsafe impl NSObjectProtocol for SidebarSource {}

    // SAFETY: the method matches the protocol's signature.
    unsafe impl NSTableViewDataSource for SidebarSource {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table: &NSTableView) -> NSInteger {
            NSInteger::try_from(self.ivars().shown.borrow().len()).unwrap_or_default()
        }
    }

    // SAFETY: the method matches the protocol's signature.
    unsafe impl NSControlTextEditingDelegate for SidebarSource {
        /// In the search field, `↓` and `⏎` go to the first pane it keeps.
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn search_command(&self, control: &NSControl, _text: &NSTextView, command: Sel) -> bool {
            let down = command == sel!(moveDown:) || command == sel!(insertNewline:);
            if down {
                // The field's action may still be on its way; what it holds now decides.
                self.filter(&control.stringValue().to_string());
                self.choose_first();
            }
            down
        }
    }

    unsafe impl NSTextFieldDelegate for SidebarSource {}

    unsafe impl NSSearchFieldDelegate for SidebarSource {}

    // SAFETY: the methods match the protocol's signatures.
    unsafe impl NSTableViewDelegate for SidebarSource {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for_row(
            &self,
            table: &NSTableView,
            _column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<NSView>> {
            self.pane_at(row)
                .map(|pane| cell(pane, table.bounds().size.width, self.mtm()))
        }

        /// The row reads as its pane: `VoiceOver` stops at rows, not at their cells.
        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row_view(
            &self,
            _table: &NSTableView,
            row: NSInteger,
        ) -> Option<Retained<NSTableRowView>> {
            let view = RowView::new(Tone::Plain, self.mtm());
            if let Some(pane) = self.pane_at(row) {
                view.setAccessibilityLabel(Some(&NSString::from_str(pane.title())));
            }
            Some(view.into_super())
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_changed(&self, notification: &NSNotification) {
            if self.ivars().quiet.get() {
                return;
            }
            let Some(table) = notification
                .object()
                .and_then(|object| object.downcast::<NSTableView>().ok())
            else {
                return;
            };
            if let Some(pane) = self.pane_at(table.selectedRow()) {
                self.choose(pane);
            }
        }
    }

    impl SidebarSource {
        #[unsafe(method(searchChanged:))]
        fn search_changed(&self, field: &NSSearchField) {
            self.filter(&field.stringValue().to_string());
        }
    }
);

impl SidebarSource {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            choose: RefCell::new(None),
            quiet: Cell::new(false),
            shown: RefCell::new(Pane::ALL.to_vec()),
            current: Cell::new(None),
            table: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn pane_at(&self, row: NSInteger) -> Option<Pane> {
        usize::try_from(row)
            .ok()
            .and_then(|row| self.ivars().shown.borrow().get(row).copied())
    }

    fn choose(&self, pane: Pane) {
        let choose = self.ivars().choose.borrow().clone();
        if let Some(choose) = choose {
            choose(pane);
        }
    }

    /// Keeps the panes that match `query`, the one on screen still selected when kept.
    fn filter(&self, query: &str) {
        let ivars = self.ivars();
        *ivars.shown.borrow_mut() = Pane::ALL
            .into_iter()
            .filter(|pane| matches(*pane, query))
            .collect();
        let table = ivars.table.borrow().clone();
        if let Some(table) = table {
            table.reloadData();
            if let Some(current) = ivars.current.get() {
                self.select(&table, current);
            }
        }
    }

    /// The first pane the search kept comes up, and the list takes the keyboard.
    fn choose_first(&self) {
        let first = self.ivars().shown.borrow().first().copied();
        let table = self.ivars().table.borrow().clone();
        let (Some(pane), Some(table)) = (first, table) else {
            return;
        };
        if let Some(window) = table.window() {
            window.makeFirstResponder(Some(&table));
        }
        self.choose(pane);
    }

    /// Shows `pane` as the selected row, when the search keeps it, without telling.
    fn select(&self, table: &NSTableView, pane: Pane) {
        let ivars = self.ivars();
        ivars.current.set(Some(pane));
        let row = ivars.shown.borrow().iter().position(|shown| *shown == pane);
        ivars.quiet.set(true);
        match row {
            Some(row) => {
                table.selectRowIndexes_byExtendingSelection(
                    &NSIndexSet::indexSetWithIndex(row),
                    false,
                );
            }
            // SAFETY: `deselectAll:` takes any sender, including none.
            None => unsafe { table.deselectAll(None) },
        }
        ivars.quiet.set(false);
    }
}

/// The sidebar: its material, the search, and the list of panes on it.
pub struct Sidebar {
    view: Retained<NSVisualEffectView>,
    search: Retained<NSSearchField>,
    table: Retained<NSTableView>,
    source: Retained<SidebarSource>,
}

impl Sidebar {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(WIDTH, HEIGHT));
        let view = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
        view.setMaterial(NSVisualEffectMaterial::Sidebar);
        view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        let source = SidebarSource::new(mtm);
        let table = NSTableView::initWithFrame(NSTableView::alloc(mtm), NSRect::ZERO);
        let column = NSTableColumn::initWithIdentifier(
            NSTableColumn::alloc(mtm),
            &NSString::from_str("pane"),
        );
        column.setWidth(WIDTH - space::SM * 2.0);
        table.addTableColumn(&column);
        table.setHeaderView(None);
        // Plain, not a source list: the row view draws the selection as the palette does.
        table.setStyle(NSTableViewStyle::Plain);
        table.setBackgroundColor(&NSColor::clearColor());
        table.setIntercellSpacing(NSSize::new(0.0, 0.0));
        table.setRowHeight(ROW_HEIGHT);
        table.setAccessibilityLabel(Some(&NSString::from_str("Settings panes")));
        // SAFETY: the table holds its data source and delegate weakly; the sidebar keeps the
        // source alive for as long as the table.
        unsafe { table.setDataSource(Some(ProtocolObject::from_ref(&*source))) };
        // SAFETY: as above.
        unsafe { table.setDelegate(Some(ProtocolObject::from_ref(&*source))) };
        *source.ivars().table.borrow_mut() = Some(table.clone());
        let search = NSSearchField::initWithFrame(
            NSSearchField::alloc(mtm),
            NSRect::new(
                NSPoint::new(space::MD, HEIGHT - Page::HEADER - SEARCH_HEIGHT),
                NSSize::new(WIDTH - space::MD * 2.0, SEARCH_HEIGHT),
            ),
        );
        search.setPlaceholderString(Some(&NSString::from_str("Search settings")));
        search.setSendsSearchStringImmediately(true);
        // SAFETY: the field holds its delegate weakly; the sidebar keeps the source alive as
        // long as the field.
        unsafe { search.setDelegate(Some(ProtocolObject::from_ref(&*source))) };
        // SAFETY: as above, and the source implements `searchChanged:`.
        unsafe { search.setTarget(Some(&source)) };
        // SAFETY: as above.
        unsafe { search.setAction(Some(sel!(searchChanged:))) };
        view.addSubview(&search);
        let list_height = HEIGHT - Page::HEADER - SEARCH_HEIGHT - space::MD;
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            NSRect::new(
                NSPoint::new(space::SM, 0.0),
                NSSize::new(WIDTH - space::SM * 2.0, list_height),
            ),
        );
        scroll.setDocumentView(Some(&table));
        scroll.setDrawsBackground(false);
        scroll.setBorderType(NSBorderType::NoBorder);
        scroll.setHasVerticalScroller(false);
        view.addSubview(&scroll);
        table.sizeLastColumnToFit();
        Self {
            view,
            search,
            table,
            source,
        }
    }

    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// The list, the window's first stop for the keyboard.
    pub fn list(&self) -> &NSView {
        &self.table
    }

    /// The search field above the list.
    pub fn search(&self) -> &NSSearchField {
        &self.search
    }

    /// Runs `choose` with the pane the user picks.
    pub fn on_choose(&self, choose: impl Fn(Pane) + 'static) {
        *self.source.ivars().choose.borrow_mut() = Some(Rc::new(choose));
    }

    /// Shows `pane` as the selected one, without telling.
    pub fn select(&self, pane: Pane) {
        self.source.select(&self.table, pane);
    }
}

/// Whether `pane` answers `query`: every word of it starts a word of the pane's name or of
/// what it sets, ignoring case. An empty query keeps every pane.
fn matches(pane: Pane, query: &str) -> bool {
    let haystack = format!("{} {}", pane.title(), keywords(pane)).to_lowercase();
    let words: Vec<&str> = haystack
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|wanted| words.iter().any(|word| word.starts_with(wanted)))
}

/// What each pane sets, in the words a search would use.
fn keywords(pane: Pane) -> &'static str {
    match pane {
        Pane::General => "default browser downloads folder",
        Pane::Shields => {
            "privacy shields filter lists trackers ads blocking https javascript protection"
        }
        Pane::Extensions => "extensions add-ons web store",
        Pane::Keys => "keyboard shortcuts keys keymap keybindings",
        Pane::Agent => "agent llm model endpoint ollama api key local",
    }
}

/// A pane's row: its symbol and its title. The selection is a light accent tint under them
/// (`selection-fill`), so both keep their colors. `VoiceOver` reads the pane's name.
fn cell(pane: Pane, width: CGFloat, mtm: MainThreadMarker) -> Retained<NSView> {
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(width, ROW_HEIGHT)),
    );
    view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    view.setAccessibilityElement(true);
    view.setAccessibilityLabel(Some(&NSString::from_str(pane.title())));
    let title = NSTextField::labelWithString(&NSString::from_str(pane.title()), mtm);
    title.setFont(Some(&TextRole::RowTitle.font()));
    title.sizeToFit();
    let icon_size = TextRole::Headline.point_size() + space::SM;
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(pane.symbol()),
        None,
    )
    .and_then(|image| {
        let configuration = NSImageSymbolConfiguration::configurationWithPointSize_weight(
            TextRole::RowTitle.point_size(),
            0.0,
        );
        image.imageWithSymbolConfiguration(&configuration)
    });
    let icon = NSImageView::initWithFrame(
        NSImageView::alloc(mtm),
        NSRect::new(
            NSPoint::new(space::MD, ((ROW_HEIGHT - icon_size) / 2.0).round()),
            NSSize::new(icon_size, icon_size),
        ),
    );
    icon.setImage(image.as_deref());
    let tint = if pane == Pane::Agent {
        color::agent()
    } else {
        color::accent()
    };
    icon.setContentTintColor(Some(&tint));
    let title_height = title.frame().size.height;
    title.setFrame(NSRect::new(
        NSPoint::new(
            space::MD + icon_size + space::SM,
            ((ROW_HEIGHT - title_height) / 2.0).round(),
        ),
        NSSize::new(
            width - icon_size - space::MD * 2.0 - space::SM,
            title_height,
        ),
    ));
    title.setTextColor(Some(&color::label()));
    view.addSubview(&icon);
    view.addSubview(&title);
    view
}

#[cfg(test)]
mod tests {
    use super::matches;
    use crate::settings::Pane;

    #[test]
    fn a_search_keeps_the_panes_whose_names_or_settings_match() {
        let kept = |query: &str| -> Vec<Pane> {
            Pane::ALL
                .into_iter()
                .filter(|pane| matches(*pane, query))
                .collect()
        };

        assert_eq!(kept(""), Pane::ALL);
        assert_eq!(kept("key"), [Pane::Keys, Pane::Agent]);
        assert_eq!(kept("Ollama"), [Pane::Agent]);
        assert_eq!(kept("filter lists"), [Pane::Shields]);
        assert_eq!(kept("down"), [Pane::General]);
        assert_eq!(kept("nothing here"), Vec::<Pane>::new());
    }
}
