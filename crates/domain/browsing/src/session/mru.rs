//! Most-recently-used order of one window's tabs; the front is the active tab.

use quickie_domain_kernel::TabId;

/// The tabs of one window, most recent first. Membership changes only through `push_*` and
/// `remove`; `touch` only reorders.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MruStack(Vec<TabId>);

impl MruStack {
    pub(crate) fn from_order(order: Vec<TabId>) -> Self {
        Self(order)
    }

    pub(crate) fn push_front(&mut self, id: TabId) {
        self.0.insert(0, id);
    }

    pub(crate) fn push_back(&mut self, id: TabId) {
        self.0.push(id);
    }

    /// Moves a member to the front. `false` if it is not a member.
    pub(crate) fn touch(&mut self, id: TabId) -> bool {
        let Some(at) = self.position(id) else {
            return false;
        };
        self.0[..=at].rotate_right(1);
        true
    }

    /// Removes a member. `false` if it is not a member.
    pub(crate) fn remove(&mut self, id: TabId) -> bool {
        let Some(at) = self.position(id) else {
            return false;
        };
        self.0.remove(at);
        true
    }

    pub(crate) fn contains(&self, id: TabId) -> bool {
        self.0.contains(&id)
    }

    pub(crate) fn front(&self) -> Option<TabId> {
        self.0.first().copied()
    }

    pub(crate) fn as_slice(&self) -> &[TabId] {
        &self.0
    }

    fn position(&self, id: TabId) -> Option<usize> {
        self.0.iter().position(|&member| member == id)
    }
}

#[cfg(test)]
mod tests {
    use super::MruStack;
    use quickie_domain_kernel::{Profile, TabId};

    fn id(serial: u64) -> TabId {
        TabId::new(Profile::Main, serial)
    }

    fn stack(raw: &[u64]) -> MruStack {
        MruStack::from_order(raw.iter().copied().map(id).collect())
    }

    #[test]
    fn touch_moves_a_member_to_the_front_and_keeps_the_rest() {
        let mut mru = stack(&[1, 2, 3, 4]);

        let touched = mru.touch(id(3));

        assert!(touched);
        assert_eq!(mru, stack(&[3, 1, 2, 4]));
    }

    #[test]
    fn touch_ignores_strangers() {
        let mut mru = stack(&[1, 2]);

        let touched = mru.touch(id(9));

        assert!(!touched);
        assert_eq!(mru, stack(&[1, 2]));
    }

    #[test]
    fn remove_drops_only_that_member() {
        let mut mru = stack(&[1, 2, 3]);

        assert!(mru.remove(id(2)));
        assert!(!mru.remove(id(2)));
        assert_eq!(mru, stack(&[1, 3]));
    }

    #[test]
    fn pushes_go_to_their_ends() {
        let mut mru = stack(&[1]);

        mru.push_front(id(2));
        mru.push_back(id(3));

        assert_eq!(mru.front(), Some(id(2)));
        assert_eq!(mru.as_slice(), stack(&[2, 1, 3]).as_slice());
        assert!(mru.contains(id(3)));
        assert!(!mru.contains(id(4)));
    }
}
