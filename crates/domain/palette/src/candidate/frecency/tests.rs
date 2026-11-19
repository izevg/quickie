use quickie_domain_kernel::Timestamp;

use super::{Frecency, FrecencyError};

const NOW: Timestamp = Timestamp::from_millis(1_000 * Timestamp::DAY_MS);

/// A visit `days` days and `extra_ms` milliseconds before `NOW`.
fn ago(days: u64, extra_ms: u64) -> Timestamp {
    Timestamp::from_millis(NOW.as_millis() - days * Timestamp::DAY_MS - extra_ms)
}

/// The score of a single visit, which equals that visit's weight.
fn one_visit(at: Timestamp) -> u64 {
    Frecency::from_visits([at]).score(NOW)
}

#[test]
fn a_candidate_never_visited_scores_zero() {
    assert_eq!(Frecency::default().score(NOW), 0);
}

#[test]
fn a_count_without_known_visit_times_scores_zero() {
    let frecency = Frecency::sampled(7, &[]).unwrap();

    assert_eq!(frecency.score(NOW), 0);
}

#[test]
fn a_visit_up_to_four_days_old_weighs_100() {
    assert_eq!(one_visit(ago(0, 0)), 100);
    assert_eq!(one_visit(ago(4, 0)), 100);
}

#[test]
fn a_visit_up_to_fourteen_days_old_weighs_70() {
    assert_eq!(one_visit(ago(4, 1)), 70);
    assert_eq!(one_visit(ago(14, 0)), 70);
}

#[test]
fn a_visit_up_to_a_month_old_weighs_50() {
    assert_eq!(one_visit(ago(14, 1)), 50);
    assert_eq!(one_visit(ago(31, 0)), 50);
}

#[test]
fn a_visit_up_to_ninety_days_old_weighs_30() {
    assert_eq!(one_visit(ago(31, 1)), 30);
    assert_eq!(one_visit(ago(90, 0)), 30);
}

#[test]
fn an_older_visit_weighs_10() {
    assert_eq!(one_visit(ago(90, 1)), 10);
}

#[test]
fn a_visit_after_now_counts_as_brand_new() {
    let later = Timestamp::from_millis(NOW.as_millis() + Timestamp::DAY_MS);

    assert_eq!(one_visit(later), 100);
}

#[test]
fn the_visit_count_scales_the_average_weight() {
    let frecency = Frecency::sampled(4, &[ago(0, 0), ago(1, 0), ago(20, 0)]).unwrap();

    // ceil(4 × (100 + 100 + 50) / 3) = ceil(333.3…)
    assert_eq!(frecency.score(NOW), 334);
}

#[test]
fn every_recorded_visit_counts() {
    let frecency = Frecency::from_visits([ago(1, 0), ago(2, 0), ago(3, 0)]);

    assert_eq!(frecency.visit_count(), 3);
    assert_eq!(frecency.score(NOW), 300);
}

/// Ten visits within the last four days, each a minute apart.
fn ten_fresh_visits() -> impl Iterator<Item = Timestamp> {
    (0..10).map(|minute| ago(0, minute * 60_000))
}

#[test]
fn only_the_newest_visits_are_sampled() {
    let mut visits: Vec<Timestamp> = ten_fresh_visits().collect();
    visits.insert(0, ago(200, 0));

    let frecency = Frecency::from_visits(visits);

    // 11 × (10 × 100) / 10: the old visit adds to the count but not to the weights.
    assert_eq!(frecency.score(NOW), 1_100);
}

#[test]
fn a_late_recorded_old_visit_does_not_push_out_a_newer_one() {
    let mut frecency = Frecency::from_visits(ten_fresh_visits());

    frecency.record(ago(200, 0));

    assert_eq!(frecency.score(NOW), 1_100);
}

#[test]
fn a_sampled_history_keeps_its_newest_visits() {
    let mut recent: Vec<Timestamp> = ten_fresh_visits().collect();
    recent.insert(0, ago(200, 0));

    let frecency = Frecency::sampled(11, &recent).unwrap();

    assert_eq!(frecency.score(NOW), 1_100);
}

#[test]
fn a_sampled_history_may_have_as_many_samples_as_visits() {
    let frecency = Frecency::sampled(2, &[ago(0, 0), ago(0, 0)]).unwrap();

    assert_eq!((frecency.visit_count(), frecency.score(NOW)), (2, 200));
}

#[test]
fn a_sampled_history_rejects_more_samples_than_visits() {
    let result = Frecency::sampled(1, &[ago(0, 0), ago(1, 0)]);

    assert_eq!(
        result,
        Err(FrecencyError::MoreSamplesThanVisits {
            samples: 2,
            visit_count: 1
        })
    );
}

#[test]
fn the_error_names_both_numbers() {
    let error = FrecencyError::MoreSamplesThanVisits {
        samples: 3,
        visit_count: 2,
    };

    assert_eq!(
        error.to_string(),
        "3 sampled visits exceed the visit count of 2"
    );
}
