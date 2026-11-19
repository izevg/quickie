//! What a page asks of the browser through its `WKUIDelegate` ([`delegate`]): a page of its own
//! (`window.open`, `target=_blank`), to close itself, a JavaScript dialog, or a device. A dialog
//! or a permission waits for the user; its [`Reply`] carries the answer back to the page.

pub(crate) mod delegate;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_app_kit::NSEventModifierFlags;
use objc2_foundation::NSString;
use objc2_web_kit::WKPermissionDecision;
use quickie_application::Permission;
use quickie_domain_browsing::Focus;

/// A JavaScript dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// `alert()`: a message and OK.
    Alert { message: String },
    /// `confirm()`: OK or Cancel.
    Confirm { message: String },
    /// `prompt()`: a line of text, OK or Cancel.
    Prompt { message: String, default: String },
}

/// What a page asks the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskKind {
    Dialog(Dialog),
    Permission(Permission),
}

/// The user's answer to an ask.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// OK, or allow; a prompt's text.
    Accept(Option<String>),
    /// Cancel, or deny: the safe default.
    Decline,
}

/// How a page waits for an answer. `WebKit` requires every reply to be sent exactly once, so a
/// reply dropped unanswered (its tab closed) declines.
pub(crate) struct Reply(Option<Pending>);

enum Pending {
    Alert(RcBlock<dyn Fn()>),
    Confirm(RcBlock<dyn Fn(Bool)>),
    Prompt(RcBlock<dyn Fn(*mut NSString)>),
    Permission(RcBlock<dyn Fn(WKPermissionDecision)>),
}

impl Reply {
    /// `alert()` waits for OK.
    pub(crate) fn alert(reply: RcBlock<dyn Fn()>) -> Self {
        Self(Some(Pending::Alert(reply)))
    }

    /// `confirm()` waits for OK (`YES`) or Cancel.
    pub(crate) fn confirm(reply: RcBlock<dyn Fn(Bool)>) -> Self {
        Self(Some(Pending::Confirm(reply)))
    }

    /// `prompt()` waits for its text, or nil for Cancel.
    pub(crate) fn prompt(reply: RcBlock<dyn Fn(*mut NSString)>) -> Self {
        Self(Some(Pending::Prompt(reply)))
    }

    /// A device request waits for grant or deny.
    pub(crate) fn permission(reply: RcBlock<dyn Fn(WKPermissionDecision)>) -> Self {
        Self(Some(Pending::Permission(reply)))
    }

    pub(crate) fn answer(mut self, answer: &Answer) {
        if let Some(pending) = self.0.take() {
            pending.send(answer);
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if let Some(pending) = self.0.take() {
            pending.send(&Answer::Decline);
        }
    }
}

impl Pending {
    fn send(self, answer: &Answer) {
        let accepted = matches!(answer, Answer::Accept(_));
        match self {
            Self::Alert(reply) => reply.call(()),
            Self::Confirm(reply) => reply.call((Bool::new(accepted),)),
            Self::Prompt(reply) => match answer {
                Answer::Accept(text) => {
                    let text = NSString::from_str(text.as_deref().unwrap_or_default());
                    reply.call((Retained::as_ptr(&text).cast_mut(),));
                }
                Answer::Decline => reply.call((std::ptr::null_mut(),)),
            },
            Self::Permission(reply) => reply.call((if accepted {
                WKPermissionDecision::Grant
            } else {
                WKPermissionDecision::Deny
            },)),
        }
    }
}

/// The mouse button numbers `WebKit` reports for a navigation action: a mask, like
/// `NSEvent.pressedMouseButtons`, with the middle button third.
const MIDDLE_BUTTON: isize = 1 << 2;

/// Where a link opened with `modifiers` and mouse `button` goes: `⌘`-click and middle-click
/// open a background tab, `⇧⌘`-click a foreground one; `None` for a plain click, which stays in
/// its page.
#[must_use]
pub(crate) fn link_focus(modifiers: NSEventModifierFlags, button: isize) -> Option<Focus> {
    let command = modifiers.contains(NSEventModifierFlags::Command);
    let shift = modifiers.contains(NSEventModifierFlags::Shift);
    match (command || button == MIDDLE_BUTTON, shift) {
        (false, _) => None,
        (true, false) => Some(Focus::Background),
        (true, true) => Some(Focus::Foreground),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_app_kit::NSEventModifierFlags;
    use objc2_foundation::NSString;
    use objc2_web_kit::WKPermissionDecision;
    use quickie_domain_browsing::Focus;

    use super::{Answer, MIDDLE_BUTTON, Reply, link_focus};

    /// What the page got back, each time it got something.
    type Heard = Rc<RefCell<Vec<String>>>;

    fn confirm(heard: &Heard) -> Reply {
        let heard = Rc::clone(heard);
        Reply::confirm(RcBlock::new(move |ok: Bool| {
            heard.borrow_mut().push(format!("confirm {}", ok.as_bool()));
        }))
    }

    fn prompt(heard: &Heard) -> Reply {
        let heard = Rc::clone(heard);
        Reply::prompt(RcBlock::new(move |text: *mut NSString| {
            // SAFETY: the reply passes a valid string or null for the duration of the call.
            let text = unsafe { text.as_ref() }.map(NSString::to_string);
            heard.borrow_mut().push(format!("prompt {text:?}"));
        }))
    }

    fn permission(heard: &Heard) -> Reply {
        let heard = Rc::clone(heard);
        Reply::permission(RcBlock::new(move |decision: WKPermissionDecision| {
            heard
                .borrow_mut()
                .push(format!("permission {}", decision.0));
        }))
    }

    #[test]
    fn each_dialog_hears_its_answer_once() {
        let heard = Heard::default();
        let alerted = Rc::new(RefCell::new(0));
        let count = Rc::clone(&alerted);

        Reply::alert(RcBlock::new(move || *count.borrow_mut() += 1)).answer(&Answer::Decline);
        confirm(&heard).answer(&Answer::Accept(None));
        confirm(&heard).answer(&Answer::Decline);
        prompt(&heard).answer(&Answer::Accept(Some("Grace".to_owned())));
        prompt(&heard).answer(&Answer::Accept(None));
        prompt(&heard).answer(&Answer::Decline);

        assert_eq!(*alerted.borrow(), 1);
        assert_eq!(
            *heard.borrow(),
            [
                "confirm true",
                "confirm false",
                "prompt Some(\"Grace\")",
                "prompt Some(\"\")",
                "prompt None"
            ]
        );
    }

    #[test]
    fn a_permission_is_granted_or_denied() {
        let heard = Heard::default();

        permission(&heard).answer(&Answer::Accept(None));
        permission(&heard).answer(&Answer::Decline);

        assert_eq!(*heard.borrow(), ["permission 1", "permission 2"]);
    }

    #[test]
    fn a_reply_dropped_unanswered_declines() {
        let heard = Heard::default();

        drop(confirm(&heard));
        drop(prompt(&heard));
        drop(permission(&heard));

        assert_eq!(
            *heard.borrow(),
            ["confirm false", "prompt None", "permission 2"]
        );
    }

    const LEFT: isize = 1;

    #[test]
    fn a_plain_click_stays_in_the_page() {
        assert_eq!(link_focus(NSEventModifierFlags::empty(), LEFT), None);
        assert_eq!(link_focus(NSEventModifierFlags::Shift, LEFT), None);
        assert_eq!(link_focus(NSEventModifierFlags::Option, 0), None);
    }

    #[test]
    fn command_or_middle_click_opens_a_background_tab() {
        assert_eq!(
            link_focus(NSEventModifierFlags::Command, LEFT),
            Some(Focus::Background)
        );
        assert_eq!(
            link_focus(NSEventModifierFlags::empty(), MIDDLE_BUTTON),
            Some(Focus::Background)
        );
    }

    #[test]
    fn shift_brings_it_to_the_front() {
        let both = NSEventModifierFlags::Command | NSEventModifierFlags::Shift;

        assert_eq!(link_focus(both, LEFT), Some(Focus::Foreground));
        assert_eq!(
            link_focus(NSEventModifierFlags::Shift, MIDDLE_BUTTON),
            Some(Focus::Foreground)
        );
    }
}
