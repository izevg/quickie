//! The empty state (`docs/design/mocks/agent/ask-empty/`): no endpoint is set up, so the panel
//! says what the agent needs, centred under a tile in the agent color, and offers the settings
//! (filled orange) and looking for a local server.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSTextAlignment, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::{TextRole, color, radius, space};
use quickie_ui_shared::{Emphasis, Plate, SheetAnswer, keyed_button, symbol};

use super::{agent_fill, agent_line};

/// The tile's side.
const TILE: CGFloat = 48.0;

/// The empty state's views, in a flipped column.
pub struct Empty {
    column: Retained<Plate>,
    emblem: Retained<Plate>,
    headline: Retained<NSTextField>,
    body: Retained<NSTextField>,
    settings: Retained<NSView>,
    detect: Retained<NSView>,
}

impl Empty {
    /// The state; its buttons send `settings:` and `detect:` to `target`.
    ///
    /// # Safety
    ///
    /// `target` must implement `settings:` and `detect:`, each taking the sender, and outlive
    /// the state.
    pub unsafe fn new(target: &AnyObject, mtm: MainThreadMarker) -> Self {
        let column = Plate::new(None, None, 0.0, mtm);
        let emblem = Plate::new(Some(agent_fill), Some(agent_line), radius::ROW + 2.0, mtm);
        emblem.setFrameSize(NSSize::new(TILE, TILE));
        if let Some(glyph) = symbol("brain", "", TextRole::Title, &color::agent(), mtm) {
            let size = glyph.frame().size;
            glyph.setFrameOrigin(NSPoint::new(
                ((TILE - size.width) / 2.0).round(),
                ((TILE - size.height) / 2.0).round(),
            ));
            emblem.addSubview(&glyph);
        }
        let headline =
            NSTextField::labelWithString(&NSString::from_str("No LLM endpoint set up"), mtm);
        headline.setFont(Some(&TextRole::Headline.font()));
        headline.setTextColor(Some(&color::label()));
        headline.setAlignment(NSTextAlignment::Center);
        let body = NSTextField::wrappingLabelWithString(
            &NSString::from_str(
                "The agent answers through an OpenAI-compatible endpoint you choose: a local \
                 server such as Ollama or LM Studio, or a hosted API. Quickie sends the page \
                 only when you ask, and only to that endpoint.",
            ),
            mtm,
        );
        body.setFont(Some(&TextRole::RowDetail.font()));
        body.setTextColor(Some(&color::secondary_label()));
        body.setAlignment(NSTextAlignment::Center);
        body.setSelectable(false);
        // SAFETY: the caller promises `target` implements `settings:` and outlives the state.
        let settings = unsafe {
            button(
                "Agent Settings…",
                Emphasis::Agent,
                Some("gearshape"),
                target,
                sel!(settings:),
                mtm,
            )
        };
        // SAFETY: the caller promises `target` implements `detect:` and outlives the state.
        let detect = unsafe {
            button(
                "Detect Local LLM",
                Emphasis::Plain,
                None,
                target,
                sel!(detect:),
                mtm,
            )
        };
        for view in [&*emblem as &NSView, &headline, &body, &settings, &detect] {
            column.addSubview(view);
        }
        Self {
            column,
            emblem,
            headline,
            body,
            settings,
            detect,
        }
    }

    /// The column to place in the panel.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.column
    }

    /// Lays the state out `width` wide, top down, centred; returns its height.
    pub fn lay_out(&self, width: CGFloat) -> CGFloat {
        let centred = |of: CGFloat| ((width - of) / 2.0).round();
        let mut y = space::LG;
        self.emblem.setFrameOrigin(NSPoint::new(centred(TILE), y));
        y += TILE + space::MD;
        self.headline.sizeToFit();
        let title = self.headline.frame().size.height;
        self.headline
            .setFrame(NSRect::new(NSPoint::new(0.0, y), NSSize::new(width, title)));
        y += title + space::XS;
        let text_width = (width - space::XL * 2.0).max(0.0);
        self.body.setPreferredMaxLayoutWidth(text_width);
        let body = self.body.fittingSize().height.ceil();
        self.body.setFrame(NSRect::new(
            NSPoint::new(centred(text_width), y),
            NSSize::new(text_width, body),
        ));
        y += body + space::LG;
        let settings = self.settings.frame().size;
        let detect = self.detect.frame().size;
        let left = centred(settings.width + space::SM + detect.width);
        self.settings.setFrameOrigin(NSPoint::new(left, y));
        self.detect
            .setFrameOrigin(NSPoint::new(left + settings.width + space::SM, y));
        y += settings.height.max(detect.height) + space::LG;
        self.column.setFrameSize(NSSize::new(width, y));
        y
    }
}

/// # Safety
///
/// `target` must implement `action`, taking the sender, and outlive the button.
unsafe fn button(
    label: &str,
    emphasis: Emphasis,
    glyph: Option<&'static str>,
    target: &AnyObject,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let answer = SheetAnswer {
        label: label.to_owned(),
        keys: String::new(),
        emphasis,
        symbol: glyph,
    };
    // SAFETY: the caller promises `target` implements `action` and outlives the button.
    unsafe { keyed_button(&answer, None, target, action, 0, mtm) }
}
