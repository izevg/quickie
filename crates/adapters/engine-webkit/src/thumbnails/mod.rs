//! The tab overview's pictures: one per live page, at most the size the overview draws a card
//! at, cropped to the card's shape. Pure, so what is kept and what is dropped is tested without
//! `WebKit`.

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_domain_browsing::TabId;

/// The last picture of each live page. A picture that arrives for a page that has closed since
/// it was asked for is dropped, so a closed tab never keeps one.
#[derive(Debug)]
pub(crate) struct Thumbnails<I> {
    live: HashSet<TabId>,
    pictures: HashMap<TabId, I>,
}

impl<I> Default for Thumbnails<I> {
    fn default() -> Self {
        Self {
            live: HashSet::new(),
            pictures: HashMap::new(),
        }
    }
}

impl<I: Clone> Thumbnails<I> {
    /// The tab has a page now: its pictures are kept.
    pub(crate) fn open(&mut self, tab: TabId) {
        self.live.insert(tab);
    }

    /// The tab's page is gone: its picture goes, and any still being drawn is dropped.
    pub(crate) fn close(&mut self, tab: TabId) {
        self.live.remove(&tab);
        self.pictures.remove(&tab);
    }

    /// Keeps `picture` as the tab's newest. False, and nothing kept, when the page has closed.
    pub(crate) fn store(&mut self, tab: TabId, picture: I) -> bool {
        if !self.live.contains(&tab) {
            return false;
        }
        self.pictures.insert(tab, picture);
        true
    }

    pub(crate) fn get(&self, tab: TabId) -> Option<I> {
        self.pictures.get(&tab).cloned()
    }
}

/// The part of a page view of `size` that a picture of `shape` shows, in the view's flipped
/// coordinates (origin top left): the full width from the top when the view is at least as tall
/// as the shape, otherwise the full height, centered. `None` for a view with no area, which
/// `WebKit` would answer with an empty image.
pub(crate) fn crop(size: NSSize, shape: NSSize) -> Option<NSRect> {
    let area = |size: NSSize| size.width > 0.0 && size.height > 0.0;
    if !area(size) || !area(shape) {
        return None;
    }
    let aspect = shape.width / shape.height;
    let height = size.width / aspect;
    let rect = if height <= size.height {
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.width, height))
    } else {
        let width = size.height * aspect;
        NSRect::new(
            NSPoint::new((size.width - width) / 2.0, 0.0),
            NSSize::new(width, size.height),
        )
    };
    Some(rect)
}
