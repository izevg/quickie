use objc2_foundation::{NSPoint, NSRect, NSSize};
use quickie_domain_browsing::{Profile, TabId};

use super::{Thumbnails, crop};

fn tab(serial: u64) -> TabId {
    TabId::new(Profile::Main, serial)
}

const CARD: NSSize = NSSize::new(320.0, 200.0);

#[test]
fn a_live_page_keeps_its_newest_picture() {
    let mut thumbnails = Thumbnails::default();
    thumbnails.open(tab(1));
    assert!(thumbnails.store(tab(1), "first"));
    assert!(thumbnails.store(tab(1), "second"));
    assert_eq!(thumbnails.get(tab(1)), Some("second"));
    assert_eq!(thumbnails.get(tab(2)), None);
}

#[test]
fn closing_a_tab_drops_its_picture() {
    let mut thumbnails = Thumbnails::default();
    thumbnails.open(tab(1));
    thumbnails.open(tab(2));
    thumbnails.store(tab(1), "one");
    thumbnails.store(tab(2), "two");
    thumbnails.close(tab(1));
    assert_eq!(thumbnails.get(tab(1)), None);
    assert_eq!(thumbnails.get(tab(2)), Some("two"));
}

#[test]
fn a_picture_that_arrives_after_its_tab_closed_is_dropped() {
    let mut thumbnails = Thumbnails::default();
    thumbnails.open(tab(1));
    thumbnails.close(tab(1));
    assert!(!thumbnails.store(tab(1), "late"));
    assert_eq!(thumbnails.get(tab(1)), None);
}

#[test]
fn a_page_never_opened_keeps_nothing() {
    let mut thumbnails = Thumbnails::default();
    assert!(!thumbnails.store(tab(1), "stray"));
    assert_eq!(thumbnails.get(tab(1)), None);
}

#[test]
fn a_tall_view_is_cropped_from_the_top_at_full_width() {
    let rect = crop(NSSize::new(1600.0, 1200.0), CARD);
    assert_eq!(
        rect,
        Some(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(1600.0, 1000.0)
        ))
    );
}

#[test]
fn a_view_of_the_card_shape_is_drawn_whole() {
    let rect = crop(NSSize::new(800.0, 500.0), CARD);
    assert_eq!(
        rect,
        Some(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(800.0, 500.0)
        ))
    );
}

#[test]
fn a_wide_view_is_cropped_to_its_middle_at_full_height() {
    let rect = crop(NSSize::new(2000.0, 500.0), CARD);
    assert_eq!(
        rect,
        Some(NSRect::new(
            NSPoint::new(600.0, 0.0),
            NSSize::new(800.0, 500.0)
        ))
    );
}

#[test]
fn a_view_or_a_shape_without_area_has_no_picture() {
    assert_eq!(crop(NSSize::new(0.0, 0.0), CARD), None);
    assert_eq!(crop(NSSize::new(800.0, 0.0), CARD), None);
    assert_eq!(crop(NSSize::new(0.0, 500.0), CARD), None);
    let view = NSSize::new(800.0, 500.0);
    assert_eq!(crop(view, NSSize::new(0.0, 200.0)), None);
    assert_eq!(crop(view, NSSize::new(320.0, 0.0)), None);
}
