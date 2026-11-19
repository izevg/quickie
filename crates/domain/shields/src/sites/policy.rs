//! What Quickie does on one site.

/// The shields of one site. Every site starts at [`SitePolicy::DEFAULT`]: everything on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SitePolicy {
    /// Whether the filter lists block requests on the site's pages. Off, the site's pages load
    /// everything they ask for.
    pub shields: bool,
    /// Whether the site's pages load over https only.
    pub https_only: bool,
    /// Whether the site's pages may run JavaScript.
    pub javascript: bool,
}

impl SitePolicy {
    /// Where every site starts: shields up, HTTPS-only, JavaScript allowed.
    pub const DEFAULT: Self = Self {
        shields: true,
        https_only: true,
        javascript: true,
    };
}

impl Default for SitePolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}
