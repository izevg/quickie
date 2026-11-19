//! Frecency: how often and how recently something was chosen, as one number.

use quickie_domain_kernel::Timestamp;

/// Visit history of one candidate: the total number of visits and the newest few of them.
///
/// The score follows Firefox's frecency. Each sampled visit gets a weight by its age at `now`:
///
/// | Age (days, inclusive) | Weight |
/// |---|---|
/// | ≤ 4 | 100 |
/// | ≤ 14 | 70 |
/// | ≤ 31 | 50 |
/// | ≤ 90 | 30 |
/// | older | 10 |
///
/// `score = ceil(visit_count × sum(weights) / sampled_visits)`: the average weight of the newest
/// [`Frecency::SAMPLE`] visits, scaled by how often the candidate was visited in all. A visit
/// stamped after `now` counts as brand new. Without a sampled visit the score is zero.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Frecency {
    visit_count: u32,
    /// Newest first, at most `SAMPLE` long, never longer than `visit_count`.
    recent: Vec<Timestamp>,
}

/// Why a visit history cannot be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FrecencyError {
    /// More visit times were given than the visit count says happened.
    #[error("{samples} sampled visits exceed the visit count of {visit_count}")]
    MoreSamplesThanVisits {
        /// How many visit times were given.
        samples: usize,
        /// How many visits there were in all.
        visit_count: u32,
    },
}

/// Age limits in days, inclusive, and the weight of a visit within each.
const BUCKETS: [(u64, u64); 4] = [(4, 100), (14, 70), (31, 50), (90, 30)];

/// The weight of a visit older than every bucket.
const OLDEST_WEIGHT: u64 = 10;

impl Frecency {
    /// How many of the newest visits the score looks at; older ones only add to the count.
    pub const SAMPLE: usize = 10;

    /// The history of a candidate that was visited at each of `visits`, in any order.
    #[must_use]
    pub fn from_visits(visits: impl IntoIterator<Item = Timestamp>) -> Self {
        let mut frecency = Self::default();
        for at in visits {
            frecency.record(at);
        }
        frecency
    }

    /// The history of a candidate visited `visit_count` times in all, of which `recent` are
    /// known by time, as a store that keeps only the newest visits reports it.
    ///
    /// # Errors
    ///
    /// [`FrecencyError::MoreSamplesThanVisits`] when `recent` holds more visits than
    /// `visit_count`.
    pub fn sampled(visit_count: u32, recent: &[Timestamp]) -> Result<Self, FrecencyError> {
        let fits = u32::try_from(recent.len()).is_ok_and(|samples| samples <= visit_count);
        if !fits {
            return Err(FrecencyError::MoreSamplesThanVisits {
                samples: recent.len(),
                visit_count,
            });
        }
        let mut recent = recent.to_vec();
        keep_newest(&mut recent);
        Ok(Self {
            visit_count,
            recent,
        })
    }

    /// Adds one visit at `at`.
    pub fn record(&mut self, at: Timestamp) {
        self.visit_count = self.visit_count.saturating_add(1);
        self.recent.push(at);
        keep_newest(&mut self.recent);
    }

    /// How many visits there were in all.
    #[must_use]
    pub const fn visit_count(&self) -> u32 {
        self.visit_count
    }

    /// The frecency at `now`, by the formula on [`Frecency`].
    #[must_use]
    pub fn score(&self, now: Timestamp) -> u64 {
        let samples = self.recent.len() as u64;
        if samples == 0 {
            return 0;
        }
        let weights: u64 = self
            .recent
            .iter()
            .map(|&visit| weight(now.millis_since(visit)))
            .sum();
        (u64::from(self.visit_count) * weights).div_ceil(samples)
    }
}

/// Sorts newest first and drops everything past the sample.
fn keep_newest(visits: &mut Vec<Timestamp>) {
    visits.sort_unstable_by_key(|&at| std::cmp::Reverse(at));
    visits.truncate(Frecency::SAMPLE);
}

/// The weight of one visit that is `age_ms` old.
fn weight(age_ms: u64) -> u64 {
    BUCKETS
        .iter()
        .find(|&&(days, _)| age_ms <= days * Timestamp::DAY_MS)
        .map_or(OLDEST_WEIGHT, |&(_, weight)| weight)
}

#[cfg(test)]
mod tests;
