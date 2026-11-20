//! The query field of an E2 overlay: one borderless line the user types into, which reports
//! every edit. The command palette and the address overlay are each one of these above a list.
//!
//! The field keeps the keyboard while its overlay is open; the browser routes `↑` `↓` `⏎` `⎋`
//! before the field sees them, and every other key edits the text as in any text field.

mod delegate;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSFocusRingType, NSFont, NSTextField, NSTextInputClient, NSTextView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSRange, NSString};

use crate::tokens::color;
use delegate::FieldDelegate;

pub struct QueryField {
    field: Retained<NSTextField>,
    delegate: Retained<FieldDelegate>,
}

impl QueryField {
    /// A field set in `font`, showing `placeholder` while empty.
    #[must_use]
    pub fn new(font: &NSFont, placeholder: &str, mtm: MainThreadMarker) -> Self {
        let field = NSTextField::new(mtm);
        field.setFont(Some(font));
        field.setTextColor(Some(&color::label()));
        field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(false);
        field.setFocusRingType(NSFocusRingType::None);
        field.setUsesSingleLineMode(true);
        if let Some(cell) = field.cell() {
            cell.setScrollable(true);
            cell.setWraps(false);
        }
        field.sizeToFit();
        let delegate = FieldDelegate::new(mtm);
        // SAFETY: the field holds its delegate weakly; `QueryField` keeps it alive as long as
        // the field.
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        Self { field, delegate }
    }

    /// The field, to add to an overlay and give the keyboard.
    #[must_use]
    pub fn view(&self) -> &NSTextField {
        &self.field
    }

    /// The height of one line of the field.
    #[must_use]
    pub fn height(&self) -> CGFloat {
        self.field.frame().size.height
    }

    /// What the field holds.
    #[must_use]
    pub fn text(&self) -> String {
        self.field.stringValue().to_string()
    }

    /// Replaces the text, the caret after it.
    pub fn set_text(&self, text: &str) {
        self.field.setStringValue(&NSString::from_str(text));
        self.caret_to_end();
    }

    /// Replaces the text with `text`, selecting everything after its first `kept` characters
    /// (UTF-16 units, as `AppKit` counts), so typing goes on over the selection.
    pub fn set_text_selecting_after(&self, text: &str, kept: usize) {
        self.field.setStringValue(&NSString::from_str(text));
        if let Some(editor) = self.field.currentEditor() {
            let end = self.field.stringValue().length();
            let start = kept.min(end);
            editor.setSelectedRange(NSRange::new(start, end - start));
        }
    }

    /// Selects all of the text, ready to be typed over.
    pub fn select_all(&self) {
        // SAFETY: `selectText:` accepts any sender, including none.
        unsafe { self.field.selectText(None) };
    }

    /// Puts the caret after the text.
    pub fn caret_to_end(&self) {
        if let Some(editor) = self.field.currentEditor() {
            let end = self.field.stringValue().length();
            editor.setSelectedRange(NSRange::new(end, 0));
        }
    }

    /// True when nothing is selected and the caret is after the last character.
    #[must_use]
    pub fn caret_at_end(&self) -> bool {
        self.field.currentEditor().is_some_and(|editor| {
            let selected = editor.selectedRange();
            selected.length == 0 && selected.location == self.field.stringValue().length()
        })
    }

    /// True while an input method is composing in the field: `⏎` then commits the composition.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.field
            .currentEditor()
            .and_then(|editor| editor.downcast::<NSTextView>().ok())
            .is_some_and(|editor| editor.hasMarkedText())
    }

    /// Runs `changed` after each edit the user makes.
    pub fn on_change(&self, changed: impl Fn() + 'static) {
        self.delegate.on_change(changed);
    }
}
