//! Typing in the overview narrows the cards on screen: a card stays when every word typed
//! appears in its title or host, ignoring case. It filters what is shown, nothing more; the
//! tabs themselves are the browser's.

/// The positions, in order, of the `(title, host)` cards that `query` keeps.
#[must_use]
pub fn kept<'a>(cards: impl IntoIterator<Item = (&'a str, &'a str)>, query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    cards
        .into_iter()
        .enumerate()
        .filter(|(_, (title, host))| {
            let (title, host) = (title.to_lowercase(), host.to_lowercase());
            words
                .iter()
                .all(|word| title.contains(word.as_str()) || host.contains(word.as_str()))
        })
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::kept;

    const CARDS: [(&str, &str); 4] = [
        ("Sovereign Attention", "theatlantic.com"),
        ("SwiftUI — LiquidGlass", "developer.apple.com"),
        ("Погода в Москве", "yandex.ru"),
        ("apple/darwin-webkit", "github.com"),
    ];

    #[test]
    fn nothing_typed_keeps_every_card() {
        assert_eq!(kept(CARDS, ""), [0, 1, 2, 3]);
        assert_eq!(kept(CARDS, "   "), [0, 1, 2, 3]);
    }

    #[test]
    fn a_word_matches_title_or_host_ignoring_case() {
        assert_eq!(kept(CARDS, "APPLE"), [1, 3]);
        assert_eq!(kept(CARDS, "atlantic"), [0]);
        assert_eq!(kept(CARDS, "погода"), [2]);
    }

    #[test]
    fn every_word_must_match_somewhere() {
        assert_eq!(kept(CARDS, "apple github"), [3]);
        assert_eq!(kept(CARDS, "swift developer"), [1]);
        assert_eq!(kept(CARDS, "apple nowhere"), Vec::<usize>::new());
    }
}
