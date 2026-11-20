//! Zoom levels: Safari's steps.

use std::fmt;

/// Every zoom a page can have, in percent, as Safari steps through them.
const STEPS: [u16; 11] = [50, 75, 85, 100, 115, 125, 150, 175, 200, 250, 300];

/// Where [`STEPS`] has 100 %.
const ACTUAL_SIZE: usize = 3;

/// One of the zoom steps, 50 % to 300 %.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoomLevel(usize);

/// Which way a zoom command goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ZoomStep {
    /// The next larger step: `⌘=`, `⌘+`.
    In,
    /// The next smaller step: `⌘-`.
    Out,
    /// Back to 100 %: `⌘0`.
    Reset,
}

impl ZoomLevel {
    /// 100 %: the page as its author made it.
    pub const DEFAULT: Self = Self(ACTUAL_SIZE);

    /// The smallest step, 50 %.
    pub const MIN: Self = Self(0);

    /// The largest step, 300 %.
    pub const MAX: Self = Self(STEPS.len() - 1);

    /// The step that is exactly `percent`, as stored; `None` for any other number.
    #[must_use]
    pub fn from_percent(percent: u16) -> Option<Self> {
        STEPS.iter().position(|&step| step == percent).map(Self)
    }

    /// The zoom in percent: `100` for actual size.
    #[must_use]
    pub const fn percent(self) -> u16 {
        STEPS[self.0]
    }

    /// The zoom as the engine's scale factor: `1.0` for actual size.
    #[must_use]
    pub fn factor(self) -> f64 {
        f64::from(self.percent()) / 100.0
    }

    /// The level one step in or out, or 100 %; the same level at either end.
    #[must_use]
    pub const fn step(self, step: ZoomStep) -> Self {
        match step {
            ZoomStep::In if self.0 < Self::MAX.0 => Self(self.0 + 1),
            ZoomStep::Out if self.0 > Self::MIN.0 => Self(self.0 - 1),
            ZoomStep::Reset => Self::DEFAULT,
            ZoomStep::In | ZoomStep::Out => self,
        }
    }
}

impl Default for ZoomLevel {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// As a toast says it: `115 %`.
impl fmt::Display for ZoomLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} %", self.percent())
    }
}
