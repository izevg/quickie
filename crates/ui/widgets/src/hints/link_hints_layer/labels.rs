//! Link-hint labels: one per target, typed on the home row, as short as the count allows, and
//! prefix-free, so a label is followed the moment its last letter is typed. Pure.

/// The home row, left hand first: the letters hints are typed with.
pub const HOME_ROW: [char; 9] = ['a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l'];

/// `count` labels over `alphabet`, earlier targets first. As many as possible are a single
/// letter; when there are more targets than letters, the last letters become prefixes of
/// two-letter labels, and so on, so no label is the start of another. An alphabet of one
/// letter makes at most one label.
#[must_use]
pub fn labels(count: usize, alphabet: &[char]) -> Vec<String> {
    let mut leaves: Vec<String> = alphabet.iter().map(char::to_string).collect();
    if alphabet.len() < 2 {
        leaves.truncate(count);
        return leaves;
    }
    while leaves.len() < count {
        let shortest = leaves.first().map_or(0, String::len);
        let Some(last) = leaves.iter().rposition(|leaf| leaf.len() == shortest) else {
            break;
        };
        let prefix = leaves.remove(last);
        leaves.extend(alphabet.iter().map(|&letter| format!("{prefix}{letter}")));
    }
    leaves.truncate(count);
    leaves
}

/// What the letters typed so far do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typed {
    /// They start some labels: keep typing.
    Narrowed,
    /// They are this label.
    Hit(usize),
    /// They start no label.
    Miss,
}

/// Matches `typed` against `labels`.
#[must_use]
pub fn narrow(labels: &[String], typed: &str) -> Typed {
    if let Some(index) = labels.iter().position(|label| label == typed) {
        return Typed::Hit(index);
    }
    if labels.iter().any(|label| label.starts_with(typed)) {
        Typed::Narrowed
    } else {
        Typed::Miss
    }
}

#[cfg(test)]
mod tests {
    use super::{HOME_ROW, Typed, labels, narrow};

    fn prefix_free(labels: &[String]) -> bool {
        labels.iter().enumerate().all(|(i, a)| {
            labels
                .iter()
                .enumerate()
                .all(|(j, b)| i == j || !b.starts_with(a.as_str()))
        })
    }

    #[test]
    fn up_to_nine_targets_get_one_home_row_letter_each() {
        assert_eq!(labels(0, &HOME_ROW), Vec::<String>::new());
        assert_eq!(labels(3, &HOME_ROW), ["a", "s", "d"]);
        assert_eq!(labels(9, &HOME_ROW).concat(), "asdfghjkl");
    }

    #[test]
    fn a_tenth_target_turns_the_last_letter_into_a_prefix() {
        assert_eq!(
            labels(10, &HOME_ROW),
            ["a", "s", "d", "f", "g", "h", "j", "k", "la", "ls"]
        );
    }

    #[test]
    fn labels_are_unique_prefix_free_and_as_short_as_the_count_allows() {
        for count in [1, 9, 10, 17, 18, 50, 80, 81, 82, 200, 729, 730] {
            let made = labels(count, &HOME_ROW);
            assert_eq!(made.len(), count, "{count}");
            assert!(prefix_free(&made), "{count}");
            let longest = made.iter().map(String::len).max().unwrap_or(0);
            // The shortest possible longest label: the least depth with 9^depth ≥ count.
            let depth = (1..8).find(|&d| 9_usize.pow(d) >= count).unwrap();
            assert_eq!(longest, usize::try_from(depth).unwrap(), "{count}");
        }
    }

    #[test]
    fn as_many_targets_as_possible_keep_a_single_letter() {
        // m singles and (9 − m) × 9 pairs must cover the count: m = 8 for 10 … 17.
        let singles = |count| {
            labels(count, &HOME_ROW)
                .iter()
                .filter(|label| label.len() == 1)
                .count()
        };
        assert_eq!(singles(10), 8);
        assert_eq!(singles(17), 8);
        assert_eq!(singles(18), 7);
        assert_eq!(singles(81), 0);
    }

    #[test]
    fn a_one_letter_alphabet_makes_at_most_one_label() {
        assert_eq!(labels(5, &['x']), ["x"]);
        assert_eq!(labels(0, &['x']), Vec::<String>::new());
        assert_eq!(labels(3, &[]), Vec::<String>::new());
    }

    #[test]
    fn typing_narrows_then_hits() {
        let made = labels(10, &HOME_ROW);

        assert_eq!(narrow(&made, ""), Typed::Narrowed);
        assert_eq!(narrow(&made, "l"), Typed::Narrowed);
        assert_eq!(narrow(&made, "ls"), Typed::Hit(9));
        assert_eq!(narrow(&made, "a"), Typed::Hit(0));
        assert_eq!(narrow(&made, "q"), Typed::Miss);
        assert_eq!(narrow(&made, "lq"), Typed::Miss);
    }
}
