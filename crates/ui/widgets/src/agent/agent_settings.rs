//! The agent settings: the endpoint's address, its model (typed, or picked from the list the
//! endpoint gives), and its API key, as a form the Settings window shows in its Agent pane: a
//! line on what it is for, the three fields grouped as System Settings groups rows, the
//! buttons under them, and how the last request went. `⏎` saves.
//!
//! The key field is write-only: it is empty whenever the form loads, its placeholder says
//! whether the Keychain holds a key, and the browser clears it as soon as the key is saved. The
//! form contacts nothing: listing models and detecting a local LLM are buttons, and the
//! browser does them only on the click. What went wrong is said in plain words, as text.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSColor, NSComboBox, NSLineBreakMode, NSSecureTextField, NSTextField,
    NSTextFieldBezelStyle, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::FormGroup;
use quickie_ui_shared::tokens::{TextRole, color, space};

/// The form's width until its owner fits it to a pane with [`AgentSettings::fit_width`].
pub const WIDTH: CGFloat = 492.0;
/// A field's height.
const FIELD_HEIGHT: CGFloat = 24.0;
/// A row of the group: a field and the space around it.
const ROW_HEIGHT: CGFloat = FIELD_HEIGHT + space::SM * 2.0;
/// The width of the labels before the fields.
const LABEL_WIDTH: CGFloat = 96.0;

/// What the fields hold.
#[derive(Clone, PartialEq, Eq)]
pub struct SettingsForm {
    /// The endpoint's base URL, as typed.
    pub base: String,
    /// The model id, as typed or picked.
    pub model: String,
    /// A new API key; empty keeps the one in the Keychain.
    pub key: String,
}

impl std::fmt::Debug for SettingsForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsForm")
            .field("base", &self.base)
            .field("model", &self.model)
            .field(
                "key",
                &if self.key.is_empty() {
                    ""
                } else {
                    "<redacted>"
                },
            )
            .finish()
    }
}

/// What the popover asks of the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsRequest {
    /// Save the endpoint, and the key when one was typed.
    Save(SettingsForm),
    /// Ask the endpoint in the form for its models.
    ListModels(SettingsForm),
    /// Look for a local LLM server on this Mac.
    DetectLocal,
    /// Delete the key from the Keychain.
    RemoveKey,
}

/// The settings as saved, when the popover opens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsSummary {
    pub base: String,
    pub model: String,
    /// The Keychain holds a key for the endpoint.
    pub has_key: bool,
}

/// How a status line reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Success,
    Problem,
}

type Handler = Rc<dyn Fn(SettingsRequest)>;

pub struct Ivars {
    handler: RefCell<Option<Handler>>,
    form: RefCell<Option<Rc<dyn Fn() -> SettingsForm>>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `SettingsTarget` implements no
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieAgentSettingsTarget"]
    #[ivars = Ivars]
    pub struct SettingsTarget;

    unsafe impl NSObjectProtocol for SettingsTarget {}

    impl SettingsTarget {
        #[unsafe(method(save:))]
        fn save(&self, _sender: Option<&NSButton>) {
            if let Some(form) = self.form() {
                self.ask(SettingsRequest::Save(form));
            }
        }

        #[unsafe(method(list:))]
        fn list(&self, _sender: Option<&NSButton>) {
            if let Some(form) = self.form() {
                self.ask(SettingsRequest::ListModels(form));
            }
        }

        #[unsafe(method(detect:))]
        fn detect(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsRequest::DetectLocal);
        }

        #[unsafe(method(removeKey:))]
        fn remove_key(&self, _sender: Option<&NSButton>) {
            self.ask(SettingsRequest::RemoveKey);
        }
    }
);

impl SettingsTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            handler: RefCell::new(None),
            form: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer and the ivars are set.
        unsafe { msg_send![super(this), init] }
    }

    fn form(&self) -> Option<SettingsForm> {
        let form = self.ivars().form.borrow().clone()?;
        Some(form())
    }

    fn ask(&self, request: SettingsRequest) {
        let handler = self.ivars().handler.borrow().clone();
        if let Some(handler) = handler {
            handler(request);
        }
    }
}

/// The fields, shared with the target, which reads them on a click.
struct Fields {
    base: Retained<NSTextField>,
    model: Retained<NSComboBox>,
    key: Retained<NSSecureTextField>,
}

impl Fields {
    fn form(&self) -> SettingsForm {
        SettingsForm {
            base: self.base.stringValue().to_string().trim().to_owned(),
            model: self.model.stringValue().to_string().trim().to_owned(),
            key: self.key.stringValue().to_string().trim().to_owned(),
        }
    }
}

pub struct AgentSettings {
    view: Retained<NSView>,
    intro: Retained<NSTextField>,
    group: FormGroup,
    labels: [Retained<NSTextField>; 3],
    fields: Rc<Fields>,
    detect: Retained<NSButton>,
    list: Retained<NSButton>,
    remove_key: Retained<NSButton>,
    save: Retained<NSButton>,
    status: Retained<NSTextField>,
    target: Retained<SettingsTarget>,
}

impl AgentSettings {
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let target = SettingsTarget::new(mtm);
        let target_object: &AnyObject = target.as_ref();
        let base = NSTextField::new(mtm);
        base.setPlaceholderString(Some(&NSString::from_str(
            "http://127.0.0.1:11434/v1 or https://…/v1",
        )));
        let model = NSComboBox::new(mtm);
        model.setPlaceholderString(Some(&NSString::from_str("model id")));
        model.setCompletes(true);
        model.setNumberOfVisibleItems(8);
        let key = NSSecureTextField::new(mtm);
        for field in [&*base, &model, &key] {
            field.setFont(Some(&TextRole::RowTitle.font()));
            field.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
            field.setUsesSingleLineMode(true);
        }
        // The window works the key view loop out from where the fields and buttons are: top
        // down through the fields, then the buttons with Full Keyboard Access.
        let fields = Rc::new(Fields { base, model, key });
        let reader = Rc::clone(&fields);
        *target.ivars().form.borrow_mut() = Some(Rc::new(move || reader.form()));
        let intro = label(TextRole::RowDetail, &color::secondary_label(), mtm);
        intro.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        intro.setMaximumNumberOfLines(0);
        set(
            &intro,
            "Any OpenAI-compatible endpoint. Quickie contacts it only when you ask, and keeps \
             the key in the Keychain.",
        );
        let labels = ["Endpoint", "Model", "API key"].map(|text| {
            let field = label(TextRole::RowTitle, &color::label(), mtm);
            set(&field, text);
            field
        });
        let status = label(TextRole::RowDetail, &color::secondary_label(), mtm);
        status.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        status.setMaximumNumberOfLines(3);
        let save = button("Save", sel!(save:), target_object, mtm);
        // `⏎` in any field saves: Save is the window's default button.
        save.setKeyEquivalent(&NSString::from_str("\r"));
        let settings = Self {
            view: NSView::new(mtm),
            intro,
            group: FormGroup::new(mtm),
            labels,
            fields,
            detect: button("Detect Local LLM", sel!(detect:), target_object, mtm),
            list: button("List Models", sel!(list:), target_object, mtm),
            remove_key: button("Remove Key", sel!(removeKey:), target_object, mtm),
            save,
            status,
            target,
        };
        let content = &settings.view;
        for view in [
            &*settings.intro as &NSView,
            settings.group.view(),
            &settings.detect,
            &settings.list,
            &settings.remove_key,
            &settings.save,
            &settings.status,
        ] {
            content.addSubview(view);
        }
        let rows = settings.group.view();
        for view in [
            &*settings.fields.base as &NSView,
            &settings.fields.model,
            &settings.fields.key,
        ] {
            rows.addSubview(view);
        }
        for view in &settings.labels {
            rows.addSubview(view);
        }
        settings.lay_out(WIDTH);
        settings
    }

    /// Lays the form out `width` wide; its height follows.
    pub fn fit_width(&self, width: CGFloat) {
        self.lay_out(width);
    }

    /// Runs `handler` with what the user asks.
    pub fn on_request(&self, handler: impl Fn(SettingsRequest) + 'static) {
        *self.target.ivars().handler.borrow_mut() = Some(Rc::new(handler));
    }

    /// The form, [`WIDTH`] wide, to put in the Settings window.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.view
    }

    /// What the fields hold now.
    #[must_use]
    pub fn form(&self) -> SettingsForm {
        self.fields.form()
    }

    /// Fills the form with the saved settings; the key field stays empty.
    pub fn load(&self, summary: &SettingsSummary) {
        set(&self.fields.base, &summary.base);
        set(&self.fields.model, &summary.model);
        self.key_saved(summary.has_key);
        self.set_status("", Tone::Neutral);
    }

    /// The form went away: a key typed and not saved is forgotten.
    pub fn forget_key(&self) {
        self.fields.key.setStringValue(&NSString::new());
    }

    /// The key field empties, and says whether the Keychain holds a key.
    pub fn key_saved(&self, has_key: bool) {
        self.fields.key.setStringValue(&NSString::new());
        let placeholder = if has_key {
            "Saved in the Keychain; type to replace"
        } else {
            "None; a local server needs none"
        };
        self.fields
            .key
            .setPlaceholderString(Some(&NSString::from_str(placeholder)));
        self.remove_key.setEnabled(has_key);
    }

    /// Offers `models` to pick from; an empty model field takes the first.
    pub fn set_models(&self, models: &[String]) {
        self.fields.model.removeAllItems();
        for model in models {
            let value: &AnyObject = &NSString::from_str(model);
            // SAFETY: the combo box takes strings as its item values.
            unsafe { self.fields.model.addItemWithObjectValue(value) };
        }
        if self.fields.model.stringValue().length() == 0
            && let Some(first) = models.first()
        {
            set(&self.fields.model, first);
        }
    }

    /// Puts a found local server's address in the form.
    pub fn set_base(&self, base: &str) {
        set(&self.fields.base, base);
    }

    /// Says how the last request went, as plain text.
    pub fn set_status(&self, text: &str, tone: Tone) {
        set(&self.status, text);
        let tint = match tone {
            Tone::Neutral => color::secondary_label(),
            Tone::Success => color::success(),
            Tone::Problem => color::destructive(),
        };
        self.status.setTextColor(Some(&tint));
    }

    /// While a request runs, its buttons wait.
    pub fn set_busy(&self, busy: bool) {
        for button in [&self.detect, &self.list, &self.save] {
            button.setEnabled(!busy);
        }
    }

    /// Sizes the form to `width` and places everything, top down.
    fn lay_out(&self, width: CGFloat) {
        let intro_height = wrapped_height(&self.intro, width);
        let status_height = 3.0 * TextRole::RowDetail.point_size().ceil() + space::SM;
        let button_height = self.save.frame().size.height;
        let group_height = ROW_HEIGHT * 3.0;
        let height = intro_height
            + space::MD
            + group_height
            + space::LG
            + button_height
            + space::MD
            + status_height;
        self.view.setFrameSize(NSSize::new(width, height));

        let mut top = height;
        frame(&self.intro, 0.0, top - intro_height, width, intro_height);
        top -= intro_height + space::MD;
        // The group is flipped: its rows' frames run top down.
        let (rows, _) = self.group.place(
            NSPoint::new(0.0, top - group_height),
            width,
            &[ROW_HEIGHT; 3],
        );
        let fields: [&NSTextField; 3] = [&self.fields.base, &self.fields.model, &self.fields.key];
        for ((label, field), row) in self.labels.iter().zip(fields).zip(rows) {
            let label_height = label.frame().size.height;
            let middle = |height: CGFloat| row.origin.y + ((ROW_HEIGHT - height) / 2.0).round();
            frame(
                label,
                row.origin.x,
                middle(label_height),
                LABEL_WIDTH,
                label_height,
            );
            let field_left = row.origin.x + LABEL_WIDTH + space::SM;
            frame(
                field,
                field_left,
                middle(FIELD_HEIGHT),
                row.size.width - LABEL_WIDTH - space::SM,
                FIELD_HEIGHT,
            );
        }
        top -= group_height + space::LG;
        let mut x = 0.0;
        for button in [&self.detect, &self.list] {
            let size = button.frame().size;
            button.setFrameOrigin(NSPoint::new(x, top - size.height));
            x += size.width + space::SM;
        }
        let mut right = width;
        for button in [&self.save, &self.remove_key] {
            let size = button.frame().size;
            right -= size.width;
            button.setFrameOrigin(NSPoint::new(right, top - size.height));
            right -= space::SM;
        }
        top -= button_height + space::MD;
        frame(&self.status, 0.0, top - status_height, width, status_height);
    }
}

/// The height `field` needs to wrap within `width`.
fn wrapped_height(field: &NSTextField, width: CGFloat) -> CGFloat {
    field
        .cell()
        .map_or(field.frame().size.height, |cell| {
            cell.cellSizeForBounds(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(width, 1000.0),
            ))
            .height
        })
        .ceil()
}

fn frame(view: &NSView, x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) {
    view.setFrame(NSRect::new(
        NSPoint::new(x, y),
        NSSize::new(width.max(0.0), height),
    ));
}

fn set(field: &NSTextField, value: &str) {
    field.setStringValue(&NSString::from_str(value));
}

fn label(role: TextRole, tint: &NSColor, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(" "), mtm);
    field.setFont(Some(&role.font()));
    field.setTextColor(Some(tint));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.sizeToFit();
    field
}

fn button(
    title: &str,
    action: Sel,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = NSButton::new(mtm);
    button.setTitle(&NSString::from_str(title));
    button.setBezelStyle(NSBezelStyle::Push);
    button.setFont(Some(&TextRole::RowDetail.font()));
    // SAFETY: the target is weak; the form keeps it alive for as long as its buttons.
    unsafe { button.setTarget(Some(target)) };
    // SAFETY: the target implements `action`, which takes the sender.
    unsafe { button.setAction(Some(action)) };
    button.sizeToFit();
    button
}

#[cfg(test)]
mod tests {
    use super::SettingsForm;

    #[test]
    fn a_form_never_shows_its_key_in_debug_output() {
        let form = SettingsForm {
            base: "https://api.example.com/v1".to_owned(),
            model: "m".to_owned(),
            key: "sk-secret".to_owned(),
        };

        let shown = format!("{form:?}");

        assert!(!shown.contains("sk-secret"));
        assert!(shown.contains("<redacted>"));
    }
}
