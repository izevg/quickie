//! A browser-owned USB security-key prompt with the existing E3 sheet design.

use std::cell::RefCell;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::{MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSRect, NSString, ns_string};
use quickie_ui_shared::tokens::TextRole;
use quickie_ui_shared::{Emphasis, Sheet, SheetAnswer, SheetContent, SheetLine, header};

const CONTINUE: isize = 1;
const CANCEL: isize = 0;

type Completion = Box<dyn FnOnce(Option<Retained<NSString>>)>;

/// Shows a touch or PIN prompt on the page's own window. The returned closure dismisses it
/// silently. The PIN stays in an `AppKit` secure field until it is sent back to the authenticator.
pub fn show(
    parent: &NSWindow,
    origin: &str,
    relying_party: &str,
    pin: Option<(usize, bool)>,
    completed: Completion,
    mtm: MainThreadMarker,
) -> Box<dyn FnOnce()> {
    // SAFETY: a fresh main-thread allocation with standard styles and buffered backing.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::ZERO,
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: the retained window has a normal Rust owner when the native sheet ends.
    unsafe { window.setReleasedWhenClosed(false) };
    let sheet = Rc::new(Sheet::new(mtm));
    let weak_window = Weak::from_retained(&window);
    let weak_parent = Weak::from_retained(&parent.retain());
    sheet.show_in_window(
        &window,
        &content(origin, relying_party, pin),
        pin.is_some(),
        move |index| {
            if let (Some(window), Some(parent)) = (weak_window.load(), weak_parent.load())
                && parent.sheets().containsObject(&window)
            {
                parent.endSheet_returnCode(
                    &window,
                    if index == 0 && pin.is_some() {
                        CONTINUE
                    } else {
                        CANCEL
                    },
                );
            }
        },
    );
    let completion = Rc::new(RefCell::new(Some(completed)));
    let reply = Rc::clone(&completion);
    let finished = RcBlock::new(move |response| {
        let field = sheet.field();
        let value = field
            .as_ref()
            .filter(|_| response == CONTINUE)
            .map(|field| field.stringValue());
        if let Some(field) = field {
            field.setStringValue(ns_string!(""));
        }
        let completed = reply.borrow_mut().take();
        if let Some(completed) = completed {
            completed(value);
        }
    });
    parent.beginSheet_completionHandler(&window, Some(&finished));
    let parent = parent.retain();
    Box::new(move || {
        let dropped = completion.borrow_mut().take();
        if parent.sheets().containsObject(&window) {
            parent.endSheet_returnCode(&window, CANCEL);
        }
        drop(dropped);
    })
}

fn content(origin: &str, relying_party: &str, pin: Option<(usize, bool)>) -> SheetContent {
    let text = match pin {
        Some((_, true)) => "Incorrect PIN. Enter your security key PIN again.",
        Some(_) => "Enter the PIN for your security key.",
        None => "Insert your USB security key and touch it when it flashes.",
    };
    let mut lines = vec![
        line(text.to_owned(), false),
        line(format!("Security key for {relying_party}"), true),
    ];
    if let Some((remaining, _)) = pin {
        lines.push(line(format!("Attempts remaining: {remaining}"), true));
    }
    let mut answers = Vec::with_capacity(2);
    if pin.is_some() {
        answers.push(answer("Continue", "⏎", Emphasis::Primary));
    }
    answers.push(answer("Cancel", "⎋", Emphasis::Plain));
    SheetContent {
        header: header("", origin, "", ""),
        icon: None,
        lines,
        card: None,
        field: pin.map(|_| String::new()),
        answers,
    }
}

fn line(text: String, dimmed: bool) -> SheetLine {
    SheetLine {
        text,
        role: TextRole::RowTitle,
        dimmed,
    }
}

fn answer(label: &str, keys: &str, emphasis: Emphasis) -> SheetAnswer {
    SheetAnswer {
        label: label.to_owned(),
        keys: keys.to_owned(),
        emphasis,
        symbol: None,
    }
}
