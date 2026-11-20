//! The agent settings: an E2 popover at the top right with the endpoint's address, its model
//! (typed, or picked from the list the endpoint gives), and its API key.
//!
//! The key field is write-only: it is empty whenever the popover opens, its placeholder says
//! whether the Keychain holds a key, and the browser clears it as soon as the key is saved. The
//! popover contacts nothing: listing models and detecting a local LLM are buttons, and the
//! browser does them only on the click. What went wrong is said in plain words, as text.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSColor, NSComboBox, NSLineBreakMode, NSSecureTextField, NSTextField,
    NSTextFieldBezelStyle, NSTextInputClient, NSTextView, NSView, NSWindow,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, overlay, space};
use quickie_ui_shared::{Elevation, GlassSurface, Scrim, Trigger, hint_bar, motion};

/// A field's height.
const FIELD_HEIGHT: CGFloat = 24.0;
/// The width of the labels before the fields.
const LABEL_WIDTH: CGFloat = 72.0;

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
    scrim: Option<Scrim>,
    surface: GlassSurface,
    title: Retained<NSTextField>,
    intro: Retained<NSTextField>,
    labels: [Retained<NSTextField>; 3],
    fields: Rc<Fields>,
    detect: Retained<NSButton>,
    list: Retained<NSButton>,
    remove_key: Retained<NSButton>,
    save: Retained<NSButton>,
    status: Retained<NSTextField>,
    hints: Retained<NSTextField>,
    target: Retained<SettingsTarget>,
    open: Cell<bool>,
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
        let fields = Rc::new(Fields { base, model, key });
        let order: [(&NSView, &NSView); 3] = [
            (&fields.base, &fields.model),
            (&fields.model, &fields.key),
            (&fields.key, &fields.base),
        ];
        for (field, next) in order {
            // SAFETY: the key view loop runs through the popover's own fields, which the
            // popover keeps alive as long as the loop.
            unsafe { field.setNextKeyView(Some(next)) };
        }
        let reader = Rc::clone(&fields);
        *target.ivars().form.borrow_mut() = Some(Rc::new(move || reader.form()));
        let title = label(TextRole::Title, &color::label(), mtm);
        set(&title, "Agent");
        let intro = label(TextRole::RowDetail, &color::secondary_label(), mtm);
        intro.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        intro.setMaximumNumberOfLines(0);
        set(
            &intro,
            "Any OpenAI-compatible endpoint. Quickie contacts it only when you ask, and keeps \
             the key in the Keychain.",
        );
        let labels = ["Endpoint", "Model", "API key"].map(|text| {
            let field = label(TextRole::RowDetail, &color::secondary_label(), mtm);
            set(&field, text);
            field
        });
        let status = label(TextRole::RowDetail, &color::secondary_label(), mtm);
        status.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        status.setMaximumNumberOfLines(3);
        let save = button("Save", sel!(save:), target_object, mtm);
        save.setKeyEquivalent(&NSString::new());
        let settings = Self {
            scrim: Scrim::new(Elevation::E2, mtm),
            surface: GlassSurface::new(Elevation::E2, mtm),
            title,
            intro,
            labels,
            fields,
            detect: button("Detect Local LLM", sel!(detect:), target_object, mtm),
            list: button("List Models", sel!(list:), target_object, mtm),
            remove_key: button("Remove Key", sel!(removeKey:), target_object, mtm),
            save,
            status,
            hints: hint_bar(&[("⏎", "save"), ("⇥", "next field"), ("⎋", "close")], mtm),
            target,
            open: Cell::new(false),
        };
        let content = settings.surface.content_view();
        for view in [
            &*settings.title as &NSView,
            &settings.intro,
            &settings.fields.base,
            &settings.fields.model,
            &settings.fields.key,
            &settings.detect,
            &settings.list,
            &settings.remove_key,
            &settings.save,
            &settings.status,
            &settings.hints,
        ] {
            content.addSubview(view);
        }
        for view in &settings.labels {
            content.addSubview(view);
        }
        settings
    }

    /// Runs `handler` with what the user asks.
    pub fn on_request(&self, handler: impl Fn(SettingsRequest) + 'static) {
        *self.target.ivars().handler.borrow_mut() = Some(Rc::new(handler));
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// The popover's surface, for a render.
    #[must_use]
    pub fn view(&self) -> &NSView {
        self.surface.view()
    }

    /// What the fields hold now.
    #[must_use]
    pub fn form(&self) -> SettingsForm {
        self.fields.form()
    }

    /// True while one of the fields has the keyboard.
    #[must_use]
    pub fn has_focus(&self) -> bool {
        self.is_open() && self.editor().is_some()
    }

    /// True while an input method composes in a field: ⏎ then commits the composition.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.editor()
            .and_then(|editor| editor.downcast::<NSTextView>().ok())
            .is_some_and(|editor| editor.hasMarkedText())
    }

    /// Opens over `host` with the saved settings, the address field taking the keyboard.
    pub fn show(
        &self,
        host: &NSView,
        window: &NSWindow,
        summary: &SettingsSummary,
        trigger: Trigger,
    ) {
        set(&self.fields.base, &summary.base);
        set(&self.fields.model, &summary.model);
        self.key_saved(summary.has_key);
        self.set_status("", Tone::Neutral);
        self.lay_out(host);
        if !self.is_open() {
            if let Some(scrim) = &self.scrim {
                scrim.view().setFrame(host.bounds());
                host.addSubview(scrim.view());
            }
            host.addSubview(self.surface.view());
            motion::show(self.surface.view(), motion::OVERLAY_ENTER, trigger);
            self.open.set(true);
        }
        window.makeFirstResponder(Some(&self.fields.base));
    }

    pub fn hide(&self) {
        if !self.is_open() {
            return;
        }
        self.fields.key.setStringValue(&NSString::new());
        motion::hide(self.surface.view(), motion::FADE_FAST, Trigger::Keyboard);
        self.surface.view().removeFromSuperview();
        if let Some(scrim) = &self.scrim {
            scrim.view().removeFromSuperview();
        }
        self.open.set(false);
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

    fn editor(&self) -> Option<Retained<objc2_app_kit::NSText>> {
        [
            &*self.fields.base as &NSTextField,
            &self.fields.model,
            &self.fields.key,
        ]
        .into_iter()
        .find_map(|field| field.currentEditor())
    }

    /// Sizes the popover and places everything, top down.
    fn lay_out(&self, host: &NSView) {
        let padding = space::PALETTE_PADDING;
        let width = overlay::SETTINGS_WIDTH.min(host.bounds().size.width - space::XL * 2.0);
        let inner = width - padding * 2.0;
        let intro_height = wrapped_height(&self.intro, inner);
        let status_height = 3.0 * TextRole::RowDetail.point_size().ceil() + space::SM;
        let title_height = self.title.frame().size.height;
        let button_height = self.save.frame().size.height;
        let hint_height = self.hints.frame().size.height;
        let height = padding
            + title_height
            + space::XS
            + intro_height
            + space::LG
            + (FIELD_HEIGHT + space::SM) * 3.0
            + space::SM
            + button_height
            + space::MD
            + status_height
            + space::SM
            + hint_height
            + padding;
        self.surface
            .place_top_right(host, NSSize::new(width, height));

        let mut top = height - padding;
        frame(
            &self.title,
            padding,
            top - title_height,
            inner,
            title_height,
        );
        top -= title_height + space::XS;
        frame(
            &self.intro,
            padding,
            top - intro_height,
            inner,
            intro_height,
        );
        top -= intro_height + space::LG;
        let field_left = padding + LABEL_WIDTH + space::SM;
        let field_width = inner - LABEL_WIDTH - space::SM;
        let fields: [&NSTextField; 3] = [&self.fields.base, &self.fields.model, &self.fields.key];
        for (label, field) in self.labels.iter().zip(fields) {
            let label_height = label.frame().size.height;
            frame(
                label,
                padding,
                top - FIELD_HEIGHT + ((FIELD_HEIGHT - label_height) / 2.0).round(),
                LABEL_WIDTH,
                label_height,
            );
            frame(
                field,
                field_left,
                top - FIELD_HEIGHT,
                field_width,
                FIELD_HEIGHT,
            );
            top -= FIELD_HEIGHT + space::SM;
        }
        top -= space::SM;
        let mut x = padding;
        for button in [&self.detect, &self.list] {
            let size = button.frame().size;
            button.setFrameOrigin(NSPoint::new(x, top - size.height));
            x += size.width + space::SM;
        }
        let mut right = width - padding;
        for button in [&self.save, &self.remove_key] {
            let size = button.frame().size;
            right -= size.width;
            button.setFrameOrigin(NSPoint::new(right, top - size.height));
            right -= space::SM;
        }
        top -= button_height + space::MD;
        frame(
            &self.status,
            padding,
            top - status_height,
            inner,
            status_height,
        );
        frame(&self.hints, padding, padding, inner, hint_height);
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
    // SAFETY: the target is weak; the popover keeps it alive for as long as its buttons.
    unsafe { button.setTarget(Some(target)) };
    // SAFETY: the target implements `action`, which takes the sender.
    unsafe { button.setAction(Some(action)) };
    button.setRefusesFirstResponder(true);
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
