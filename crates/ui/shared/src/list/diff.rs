//! The diff behind the list's data source: which rows to remove, insert, and redraw so the
//! table turns into the new rows without reloading the ones that stayed.

/// How to turn the old rows into the new ones. Apply `removed` and `inserted` in one batch of
/// updates, then redraw `changed`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Patch {
    /// Positions in the old rows, ascending, of the rows that go.
    pub removed: Vec<usize>,
    /// Positions in the new rows, ascending, of the rows that come.
    pub inserted: Vec<usize>,
    /// Positions in the new rows, ascending, of rows that stayed but whose content changed.
    pub changed: Vec<usize>,
}

/// The patch from `old` to `new`, rows matched by `key`: the longest common run of keys stays,
/// in order, and everything else is removed or inserted.
pub fn diff<R: PartialEq, K: PartialEq>(old: &[R], new: &[R], key: impl Fn(&R) -> K) -> Patch {
    let old_keys: Vec<K> = old.iter().map(&key).collect();
    let new_keys: Vec<K> = new.iter().map(&key).collect();
    // `common[i][j]`: the longest common subsequence of `old_keys[i..]` and `new_keys[j..]`.
    let mut common = vec![vec![0_usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            common[i][j] = if old_keys[i] == new_keys[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let mut patch = Patch::default();
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if old_keys[i] == new_keys[j] {
            if old[i] != new[j] {
                patch.changed.push(j);
            }
            i += 1;
            j += 1;
        } else if common[i + 1][j] >= common[i][j + 1] {
            patch.removed.push(i);
            i += 1;
        } else {
            patch.inserted.push(j);
            j += 1;
        }
    }
    patch.removed.extend(i..old.len());
    patch.inserted.extend(j..new.len());
    patch
}

#[cfg(test)]
mod tests {
    use super::{Patch, diff};

    /// A row: its key and its content.
    type Row = (char, u8);

    fn rows(keys: &str) -> Vec<Row> {
        keys.chars().map(|key| (key, 0)).collect()
    }

    fn patch(old: &[Row], new: &[Row]) -> Patch {
        diff(old, new, |row| row.0)
    }

    /// Replays a patch the way the table does: removals at old positions, insertions at new
    /// positions, and checks the keys come out as `new`.
    fn replay(old: &[Row], new: &[Row]) {
        let patch = patch(old, new);
        let mut keys: Vec<Option<char>> = old.iter().map(|row| Some(row.0)).collect();
        for &index in patch.removed.iter().rev() {
            keys.remove(index);
        }
        for &index in &patch.inserted {
            keys.insert(index, None);
        }
        let expected: Vec<Option<char>> = new
            .iter()
            .enumerate()
            .map(|(index, row)| (!patch.inserted.contains(&index)).then_some(row.0))
            .collect();
        assert_eq!(keys, expected, "{old:?} → {new:?}");
    }

    #[test]
    fn identical_rows_need_nothing() {
        let same = rows("abc");

        assert_eq!(patch(&same, &same), Patch::default());
    }

    #[test]
    fn a_narrower_query_only_removes() {
        let patch = patch(&rows("abcd"), &rows("bd"));

        assert_eq!(patch.removed, [0, 2]);
        assert_eq!(patch.inserted, []);
    }

    #[test]
    fn new_rows_are_inserted_where_they_land() {
        let patch = patch(&rows("ac"), &rows("abcd"));

        assert_eq!(patch.inserted, [1, 3]);
        assert_eq!(patch.removed, []);
    }

    #[test]
    fn a_row_that_stays_with_new_content_is_redrawn() {
        let old = vec![('a', 0), ('b', 0)];
        let new = vec![('b', 1), ('c', 0)];

        let patch = patch(&old, &new);

        assert_eq!(patch.removed, [0]);
        assert_eq!(patch.inserted, [1]);
        assert_eq!(patch.changed, [0]);
    }

    #[test]
    fn every_patch_replays_into_the_new_rows() {
        let cases = [
            ("", "abc"),
            ("abc", ""),
            ("abc", "cba"),
            ("abcdef", "bdfxyz"),
            ("aab", "aba"),
            ("xyz", "abc"),
        ];
        for (old, new) in cases {
            replay(&rows(old), &rows(new));
        }
    }
}
