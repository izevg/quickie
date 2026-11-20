//! One page of the memory engine: where it is, where it goes next, and what it still has to do.

use std::collections::VecDeque;

use quickie_domain_browsing::{FindMatches, Jar, Video};
use quickie_domain_settings::ZoomLevel;
use url::Url;

use crate::FindStep;
use crate::conformance::{Act, TestPage};

/// What a page left behind when it showed another: it was full screen, it had a video.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Left {
    pub(super) fullscreen: bool,
    pub(super) video: bool,
}

/// A page of the memory engine. Before its tab adopts it, it is what
/// [`crate::EngineEvent::Opened`] hands over.
#[derive(Debug)]
pub struct MemoryPage {
    pub(super) jar: Jar,
    /// The URL it loads on its next turn.
    pub(super) loading: Option<Url>,
    /// The page it shows, once one committed.
    pub(super) shown: Option<(Url, TestPage)>,
    back: Vec<Url>,
    forward: Vec<Url>,
    /// What its scripts still do, one act a turn.
    pub(super) acts: VecDeque<Act>,
    /// What it asked and waits on: nothing runs until it is answered.
    pub(super) asked: Option<(crate::ReplyToken, Act)>,
    /// The selected match of the last find, counted from 1.
    found: usize,
    /// How large it is drawn, across its navigations.
    pub(super) zoom: ZoomLevel,
    /// A text field of the page it shows has focus.
    pub(super) field_focused: bool,
    /// The user just clicked it: its next act may do what only a click allows.
    pub(super) activated: bool,
    /// Its text fills its view, full screen.
    pub(super) fullscreen: bool,
    /// What the video of the page it shows does, once one played, and whether its sound is
    /// on, paused or not.
    pub(super) video: Option<(Video, bool)>,
}

impl MemoryPage {
    /// A page in `jar` that loads `url` on its first turn.
    pub(super) fn new(jar: Jar, url: Url) -> Self {
        Self {
            jar,
            loading: Some(url),
            shown: None,
            back: Vec::new(),
            forward: Vec::new(),
            acts: VecDeque::new(),
            asked: None,
            found: 0,
            zoom: ZoomLevel::DEFAULT,
            field_focused: false,
            activated: false,
            fullscreen: false,
            video: None,
        }
    }

    /// The URL it shows.
    pub(super) fn url(&self) -> Option<&Url> {
        self.shown.as_ref().map(|(url, _)| url)
    }

    /// Goes to `url`, leaving the one it shows behind.
    pub(super) fn go(&mut self, url: Url) {
        self.back.extend(self.url().cloned());
        self.forward.clear();
        self.loading = Some(url);
    }

    /// Goes back one page, when there is one.
    pub(super) fn back(&mut self) {
        if let Some(previous) = self.back.pop() {
            self.forward.extend(self.url().cloned());
            self.loading = Some(previous);
        }
    }

    /// Goes forward one page, when there is one.
    pub(super) fn forward(&mut self) {
        if let Some(next) = self.forward.pop() {
            self.back.extend(self.url().cloned());
            self.loading = Some(next);
        }
    }

    /// Starts showing `page`, at `url`: its scripts start over, nothing is found, no field
    /// has focus, nothing is full screen, and no video plays. Says what the page it showed
    /// left behind.
    pub(super) fn show(&mut self, url: Url, page: TestPage) -> Left {
        self.acts = page.on_load.iter().copied().collect();
        self.shown = Some((url, page));
        self.found = 0;
        self.field_focused = false;
        self.activated = false;
        Left {
            fullscreen: std::mem::take(&mut self.fullscreen),
            video: self.video.take().is_some(),
        }
    }

    /// Selects a match of `query` in its text as `step` says, wrapping at the ends; matching
    /// ignores case.
    pub(super) fn find(&mut self, query: &str, step: FindStep) -> FindMatches {
        let text = self
            .shown
            .as_ref()
            .map_or("", |(_, page)| page.text)
            .to_lowercase();
        let total = if query.is_empty() {
            0
        } else {
            text.matches(&query.to_lowercase()).count()
        };
        self.found = match (total, step) {
            (0, _) => 0,
            (_, FindStep::Refine) => 1,
            (_, FindStep::Next) => self.found % total + 1,
            (_, FindStep::Previous) => (self.found + total * 2 - 2) % total + 1,
        };
        FindMatches::new(Some(self.found), total)
    }
}
