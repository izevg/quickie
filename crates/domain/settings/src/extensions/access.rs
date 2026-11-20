//! What an extension may do and where: the permissions and host patterns the user granted,
//! and each site the user allowed or blocked it on.

use std::collections::BTreeSet;

use quickie_domain_kernel::Site;

/// Permissions as a manifest names them: API permissions (`storage`, `tabs`) and host match
/// patterns (`<all_urls>`, `https://*.example.com/*`). The engine checks the patterns; here
/// they are names, compared as written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grants {
    /// API permissions.
    pub permissions: BTreeSet<String>,
    /// Host match patterns.
    pub patterns: BTreeSet<String>,
}

impl Grants {
    /// Nothing granted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether nothing is in it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.permissions.is_empty() && self.patterns.is_empty()
    }

    /// What `self` has that `granted` lacks: what a request still needs the user for.
    #[must_use]
    pub fn beyond(&self, granted: &Self) -> Self {
        Self {
            permissions: self
                .permissions
                .difference(&granted.permissions)
                .cloned()
                .collect(),
            patterns: self
                .patterns
                .difference(&granted.patterns)
                .cloned()
                .collect(),
        }
    }

    /// Adds `more`; returns whether anything was new.
    pub fn add(&mut self, more: &Self) -> bool {
        let before = (self.permissions.len(), self.patterns.len());
        self.permissions.extend(more.permissions.iter().cloned());
        self.patterns.extend(more.patterns.iter().cloned());
        before != (self.permissions.len(), self.patterns.len())
    }

    /// Takes `less` away; returns whether anything was there.
    pub fn remove(&mut self, less: &Self) -> bool {
        let before = (self.permissions.len(), self.patterns.len());
        self.permissions
            .retain(|permission| !less.permissions.contains(permission));
        self.patterns
            .retain(|pattern| !less.patterns.contains(pattern));
        before != (self.permissions.len(), self.patterns.len())
    }

    /// The first name that cannot be a permission or a pattern: empty, or with white space.
    #[must_use]
    pub fn malformed(&self) -> Option<&str> {
        self.permissions
            .iter()
            .chain(&self.patterns)
            .map(String::as_str)
            .find(|name| name.is_empty() || name.chars().any(char::is_whitespace))
    }
}

/// What the user chose for an extension on one site, over what its grants say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SiteAccess {
    /// It may read and change the site's pages, even without a pattern that covers them.
    Allowed,
    /// It never runs on the site's pages, whatever its patterns cover.
    Blocked,
}

impl SiteAccess {
    /// How the store writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Blocked => "blocked",
        }
    }

    /// The access the store wrote, `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Allowed, Self::Blocked]
            .into_iter()
            .find(|access| access.as_str() == text)
    }
}

/// The match patterns that cover a site's pages and its subdomains', on any scheme the engine
/// matches: `*://example.com/*` and `*://*.example.com/*`.
#[must_use]
pub fn site_patterns(site: &Site) -> [String; 2] {
    [format!("*://{site}/*"), format!("*://*.{site}/*")]
}
