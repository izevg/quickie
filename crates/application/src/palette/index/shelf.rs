//! A shelf: the candidates of one mode, their text kept in one [`Texts`] so that gathering fifty
//! thousand pages makes no allocation per page, and ranking borrows the text from there.

use quickie_domain_palette::{Candidate, Kind, Ranked, TextSpan, Texts};
use url::Url;

use crate::palette::{PaletteItem, Target};

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
        })
    }
}
