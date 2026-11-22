use super::{Preferences, State};

#[test]
fn a_fresh_profile_never_schedules_a_check() {
    let root = tempfile::tempdir().expect("temporary directory");
    let preferences = Preferences::new(root.path().join("updates"));
    assert!(!preferences.automatic());
    assert!(!preferences.is_due());
    assert!(!root.path().join("updates").exists());
}

#[test]
fn opt_in_and_cadence_survive_a_new_process() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("updates");
    let preferences = Preferences::new(root.clone());
    preferences.set_automatic(true).expect("save");
    assert!(preferences.is_due());
    preferences.record_check().expect("save check");
    let restarted = Preferences::new(root);
    assert!(restarted.automatic());
    assert!(!restarted.is_due());
    restarted.set_automatic(false).expect("disable");
    assert!(!preferences.automatic());
    assert!(!preferences.is_due());
}

#[test]
fn a_scheduled_check_waits_a_full_day() {
    let state = State {
        automatic: true,
        checked: Some(100),
    };
    assert!(!state.due(86_499));
    assert!(state.due(86_500));
}

#[test]
fn concurrent_updaters_claim_the_same_day_only_once() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let preferences = Preferences::new(directory.path().join("updates"));
    preferences
        .set_automatic(true)
        .expect("enable automatic updates");
    let begin = std::sync::Barrier::new(2);

    let claims = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            begin.wait();
            preferences.claim_scheduled_check().expect("first claim")
        });
        let second = scope.spawn(|| {
            begin.wait();
            preferences.claim_scheduled_check().expect("second claim")
        });
        [
            first.join().expect("first worker"),
            second.join().expect("second worker"),
        ]
    });

    assert_eq!(claims.into_iter().filter(|claimed| *claimed).count(), 1);
}
