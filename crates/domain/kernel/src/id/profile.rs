//! The two profiles a tab can belong to.

use std::fmt;

/// Whose browsing a tab or a window is part of: the user's, or the agent's.
///
/// A profile has its own session, its own cookies and logins, and its own engine, so a tab
/// never moves from one profile to another. Every [`crate::TabId`] and [`crate::WindowId`]
/// names its profile, which keeps the ids of two profiles apart.
///
/// The enum is closed on purpose. The user never creates, names, or switches profiles: Quickie
/// has no user-visible profiles (throwaway jars cover what profiles do elsewhere). The two that
/// exist are fixed by the product (architecture.md §3, "Two profiles"), and every `match` on
/// one is meant to break when a third is ever added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Profile {
    /// The user's own browsing.
    Main,
    /// Where the agent acts: no cookies or logins of [`Profile::Main`].
    Agent,
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Main => "main",
            Self::Agent => "agent",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Profile;

    #[test]
    fn profiles_display_in_the_domains_words() {
        let names = [Profile::Main.to_string(), Profile::Agent.to_string()];

        assert_eq!(names, ["main", "agent"]);
    }
}
