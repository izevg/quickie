//! The sidebar: the panes as a source list on the sidebar material, under the titlebar's
//! buttons. `↑` `↓` move between panes and show each at once; a click does the same.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBorderType, NSColor, NSControlTextEditingDelegate, NSImage,
    NSImageSymbolConfiguration, NSImageView, NSScrollView, NSTableColumn, NSTableRowView,
    NSTableView, NSTableViewDataSource, NSTableViewDelegate, NSTableViewStyle, NSTextField, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectView,
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

type Choose = Rc<dyn Fn(Pane)>;

pub struct Ivars {
    choose: RefCell<Option<Choose>>,
    /// While the screen selects a row itself, the change is not told back.
    quiet: Cell<bool>,
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
            NSInteger::try_from(Pane::ALL.len()).unwrap_or_default()
        }
    }

    unsafe impl NSControlTextEditingDelegate for SidebarSource {}

    // SAFETY: the methods match the protocol's signatures.
    unsafe impl NSTableViewDelegate for SidebarSource {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for_row(
            &self,
            table: &NSTableView,
            _column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<NSView>> {
            usize::try_from(row)
                .ok()
                .and_then(|row| Pane::ALL.get(row))
                .map(|pane| cell(*pane, table.bounds().size.width, self.mtm()))
        }

        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row_view(
            &self,
            _table: &NSTableView,
            _row: NSInteger,
        ) -> Option<Retained<NSTableRowView>> {
            Some(RowView::new(Tone::Plain, self.mtm()).into_super())
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
            let pane = usize::try_from(table.selectedRow())
                .ok()
                .and_then(|row| Pane::ALL.get(row));
            let choose = self.ivars().choose.borrow().clone();
            if let (Some(pane), Some(choose)) = (pane, choose) {
                choose(*pane);
            }
        }
    }
);

impl SidebarSource {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            choose: RefCell::new(None),
            quiet: Cell::new(false),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

/// The sidebar: its material, and the list of panes on it.
pub struct Sidebar {
    view: Retained<NSVisualEffectView>,
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
        // SAFETY: the table holds its data source and delegate weakly; the sidebar keeps the
        // source alive for as long as the table.
        unsafe { table.setDataSource(Some(ProtocolObject::from_ref(&*source))) };
        // SAFETY: as above.
        unsafe { table.setDelegate(Some(ProtocolObject::from_ref(&*source))) };
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            NSRect::new(
                NSPoint::new(space::SM, 0.0),
                NSSize::new(WIDTH - space::SM * 2.0, HEIGHT - Page::HEADER),
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

    /// Runs `choose` with the pane the user picks.
    pub fn on_choose(&self, choose: impl Fn(Pane) + 'static) {
        *self.source.ivars().choose.borrow_mut() = Some(Rc::new(choose));
    }

    /// Shows `pane` as the selected one, without telling.
    pub fn select(&self, pane: Pane) {
        let ivars = self.source.ivars();
        ivars.quiet.set(true);
        self.table.selectRowIndexes_byExtendingSelection(
            &NSIndexSet::indexSetWithIndex(pane.index()),
            false,
        );
        ivars.quiet.set(false);
    }
}

/// A pane's row: its symbol and its title. The selection is a light accent tint under them
/// (`selection-fill`), so both keep their colors.
fn cell(pane: Pane, width: CGFloat, mtm: MainThreadMarker) -> Retained<NSView> {
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(width, ROW_HEIGHT)),
    );
    view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
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
