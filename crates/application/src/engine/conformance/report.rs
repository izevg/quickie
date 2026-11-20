//! What a run of the suite found, check by check.

use std::fmt;

use super::Group;

/// How one check went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The engine did what the check asks.
    Passed,
    /// It did not, for this reason.
    Failed(String),
    /// Its group is one the engine declared unsupported, for this reason.
    Skipped(&'static str),
}

/// One check and how it went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub group: Group,
    /// What the check expects, as a sentence in snake case.
    pub case: &'static str,
    pub outcome: Outcome,
}

/// Every check of a run, in the order they ran. Its `Display` is one line per check, the way
/// the test harness prints tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    verdicts: Vec<Verdict>,
}

impl Report {
    pub(super) fn new(verdicts: Vec<Verdict>) -> Self {
        Self { verdicts }
    }

    /// Every check, in the order they ran.
    #[must_use]
    pub fn verdicts(&self) -> &[Verdict] {
        &self.verdicts
    }

    /// True when no check failed; skipped ones were declared.
    #[must_use]
    pub fn passed(&self) -> bool {
        !self
            .verdicts
            .iter()
            .any(|verdict| matches!(verdict.outcome, Outcome::Failed(_)))
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for Verdict {
            group,
            case,
            outcome,
        } in &self.verdicts
        {
            write!(f, "test {group}::{case} ... ")?;
            match outcome {
                Outcome::Passed => writeln!(f, "ok")?,
                Outcome::Failed(why) => writeln!(f, "FAILED: {why}")?,
                Outcome::Skipped(why) => writeln!(f, "skipped: {why}")?,
            }
        }
        Ok(())
    }
}
