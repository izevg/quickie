//! What a toast says (`docs/design/mocks/content/toasts/`): a status symbol, what happened, a
//! count, a detail in SF Mono, and the keys that do it again or undo it. A success toast is
//! green throughout.

use quickie_ui_shared::tokens::color;
use quickie_ui_shared::{PillPart, PillTone};

/// How a toast reads at a glance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    /// Something happened that the user asked for: a tab opened in the background.
    Info,
    /// A thing is done: copied, saved, pinned.
    Done,
    /// Protection or a long task did its job: green, with its count.
    Success,
    /// It didn't work; the text says why.
    Problem,
}

/// One toast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub kind: ToastKind,
    /// What happened: `URL copied`.
    pub text: String,
    /// A measured count beside the text: `12 requests`.
    pub count: Option<String>,
    /// A detail in SF Mono: `to clipboard`, a file name.
    pub detail: Option<String>,
    /// The keys that did it, or undo it: `⇧⌘T`.
    pub keys: Option<String>,
}

impl Toast {
    /// A toast of `kind` reading `text`.
    #[must_use]
    pub fn new(kind: ToastKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            count: None,
            detail: None,
            keys: None,
        }
    }

    #[must_use]
    pub fn info(text: impl Into<String>) -> Self {
        Self::new(ToastKind::Info, text)
    }

    #[must_use]
    pub fn done(text: impl Into<String>) -> Self {
        Self::new(ToastKind::Done, text)
    }

    #[must_use]
    pub fn problem(text: impl Into<String>) -> Self {
        Self::new(ToastKind::Problem, text)
    }

    #[must_use]
    pub fn with_count(mut self, count: impl Into<String>) -> Self {
        self.count = Some(count.into());
        self
    }

    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    #[must_use]
    pub fn with_keys(mut self, keys: impl Into<String>) -> Self {
        self.keys = Some(keys.into());
        self
    }

    /// The SF Symbol before the text.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self.kind {
            ToastKind::Info => "info.circle",
            ToastKind::Done => "checkmark.circle",
            ToastKind::Success => "checkmark.shield",
            ToastKind::Problem => "exclamationmark.triangle",
        }
    }

    /// The pill's parts, left to right.
    pub(super) fn parts(&self) -> Vec<PillPart> {
        let tint = match self.kind {
            ToastKind::Info => color::accent,
            ToastKind::Done | ToastKind::Success => color::success,
            ToastKind::Problem => color::destructive,
        };
        let mut parts = vec![
            PillPart::Symbol {
                name: self.symbol(),
                tint,
            },
            PillPart::Text(self.text.clone()),
        ];
        if let Some(count) = &self.count {
            parts.push(PillPart::Count {
                text: count.clone(),
                tint: if self.kind == ToastKind::Success {
                    color::success
                } else {
                    color::secondary_label
                },
                fill: if self.kind == ToastKind::Success {
                    color::success_fill
                } else {
                    color::group_fill
                },
            });
        }
        if let Some(detail) = &self.detail {
            parts.push(PillPart::Detail(detail.clone()));
        }
        if let Some(keys) = &self.keys {
            parts.push(PillPart::Keys(keys.clone()));
        }
        parts
    }

    pub(super) fn tone(&self) -> PillTone {
        if self.kind == ToastKind::Success {
            PillTone::Success
        } else {
            PillTone::Plain
        }
    }
}

impl From<&str> for Toast {
    fn from(text: &str) -> Self {
        Self::info(text)
    }
}

impl From<String> for Toast {
    fn from(text: String) -> Self {
        Self::info(text)
    }
}

#[cfg(test)]
mod tests {
    use super::{Toast, ToastKind};

    #[test]
    fn each_kind_has_its_own_symbol() {
        let symbols = [
            ToastKind::Info,
            ToastKind::Done,
            ToastKind::Success,
            ToastKind::Problem,
        ]
        .map(|kind| Toast::new(kind, "x").symbol());

        assert_eq!(
            symbols,
            [
                "info.circle",
                "checkmark.circle",
                "checkmark.shield",
                "exclamationmark.triangle"
            ]
        );
    }

    #[test]
    fn the_parts_follow_what_the_toast_has() {
        assert_eq!(Toast::info("Tab restored").parts().len(), 2);
        let full = Toast::new(ToastKind::Success, "Shields blocked")
            .with_count("12 requests")
            .with_detail("on this page")
            .with_keys("⌥⌘S");

        assert_eq!(full.parts().len(), 5);
        assert_eq!(Toast::from("Saved"), Toast::info("Saved"));
    }
}
