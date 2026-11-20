//! A shelf: the candidates of one mode, their text kept in one [`Texts`] so that gathering fifty
//! thousand pages makes no allocation per page, and ranking borrows the text from there.

use quickie_domain_palette::{Candidate, Kind, Ranked, TextSpan, Texts};
use url::Url;

use crate::palette::{PageFacts, PaletteItem, Target};

/// What choosing a candidate on the shelf does.
#[derive(Debug)]
pub(super) enum Aim {
    /// Open the page its detail names: a bookmark or a history entry. The detail is the page's
    /// URL as the URL itself writes it, so it reads back as the same URL, and only the rows
    /// shown pay for that.
    Page,
    /// This target, with the keys bound to it (a command's); boxed, as few candidates have one.
    Fixed(Box<(Target, Vec<String>)>),
}

impl Aim {
    /// A target without keys.
    pub(super) fn at(target: Target) -> Self {
        Self::Fixed(Box::new((target, Vec::new())))
    }
}

/// One candidate, its text as spans of the shelf's texts.
#[derive(Debug)]
struct Offer {
    kind: Kind,
    title: TextSpan,
    detail: TextSpan,
    frecency: u64,
    aim: Aim,
    facts: PageFacts,
}

/// The candidates of one mode, under their position as key.
#[derive(Debug, Default)]
pub(super) struct Shelf {
    texts: Texts,
    offers: Vec<Offer>,
}

impl Shelf {
    /// Puts a candidate on the shelf. One without a title, such as a page that reported none,
    /// shows its detail (the page's URL) as title, kept once.
    pub(super) fn push(&mut self, aim: Aim, kind: Kind, title: &str, detail: &str, frecency: u64) {
        self.push_page(aim, kind, title, detail, frecency, PageFacts::default());
    }

    /// Puts a page on the shelf, with what is known of it.
    pub(super) fn push_page(
        &mut self,
        aim: Aim,
        kind: Kind,
        title: &str,
        detail: &str,
        frecency: u64,
        facts: PageFacts,
    ) {
        let detail = self.texts.keep(detail);
        let title = if title.is_empty() {
            detail.clone()
        } else {
            self.texts.keep(title)
        };
        self.offers.push(Offer {
            kind,
            title,
            detail,
            frecency,
            aim,
            facts,
        });
    }

    /// Every candidate, for one ranking.
    pub(super) fn candidates(&self) -> impl Iterator<Item = Candidate<'_, usize>> {
        self.offers
            .iter()
            .enumerate()
            .map(|(key, offer)| Candidate {
                key,
                kind: offer.kind,
                title: self.texts.get(&offer.title),
                detail: self.texts.get(&offer.detail),
                frecency: offer.frecency,
            })
    }

    /// The detail of the candidate under `key`: a page's URL on the go-to shelf.
    pub(super) fn detail(&self, key: usize) -> Option<&str> {
        let offer = self.offers.get(key)?;
        Some(self.texts.get(&offer.detail).as_str())
    }

    /// The first candidate whose detail is `detail`: on the go-to shelf, the open tab, else the
    /// bookmark, else the history entry of a page.
    pub(super) fn find(&self, detail: &str) -> Option<usize> {
        self.offers
            .iter()
            .position(|offer| self.texts.get(&offer.detail).as_str() == detail)
    }

    /// The candidate that does `target`.
    pub(super) fn find_target(&self, target: &Target) -> Option<usize> {
        self.offers
            .iter()
            .position(|offer| matches!(&offer.aim, Aim::Fixed(fixed) if fixed.0 == *target))
    }

    /// The row of a ranked candidate.
    pub(super) fn item(&self, ranked: Ranked<usize>) -> Option<PaletteItem> {
        let offer = self.offers.get(ranked.key)?;
        let detail = self.texts.get(&offer.detail).as_str();
        let (target, glyphs) = match &offer.aim {
            Aim::Page => (Target::Url(Url::parse(detail).ok()?), Vec::new()),
            Aim::Fixed(fixed) => fixed.as_ref().clone(),
        };
        Some(PaletteItem {
            target,
            kind: offer.kind,
            title: self.texts.get(&offer.title).as_str().to_owned(),
            detail: detail.to_owned(),
            glyphs,
            title_matches: ranked.title_matches,
            detail_matches: ranked.detail_matches,
            facts: offer.facts,
        })
    }
}
