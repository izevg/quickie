use quickie_domain_kernel::Site;
use url::Url;

use super::{CorruptZooms, SiteZooms, SiteZoomsSnapshot, ZoomLevel, ZoomStep};

fn site(text: &str) -> Site {
    Site::try_from(text).unwrap()
}

fn level(percent: u16) -> ZoomLevel {
    ZoomLevel::from_percent(percent).unwrap()
}

mod levels {
    use super::*;

    const SAFARI: [u16; 11] = [50, 75, 85, 100, 115, 125, 150, 175, 200, 250, 300];

    #[test]
    fn the_steps_are_safari_s() {
        let mut stepped = vec![ZoomLevel::MIN.percent()];
        let mut at = ZoomLevel::MIN;
        // Bounded, so a step that never reaches the top fails instead of looping.
        for _ in 1..SAFARI.len() {
            at = at.step(ZoomStep::In);
            stepped.push(at.percent());
        }
        assert_eq!(at, ZoomLevel::MAX);

        assert_eq!(stepped, SAFARI);
    }

    #[test]
    fn a_page_starts_at_actual_size() {
        assert_eq!(ZoomLevel::DEFAULT.percent(), 100);
        assert_eq!(ZoomLevel::default(), ZoomLevel::DEFAULT);
        assert!((ZoomLevel::DEFAULT.factor() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_factor_is_the_percent_over_a_hundred() {
        assert!((level(150).factor() - 1.5).abs() < f64::EPSILON);
        assert!((level(85).factor() - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn zooming_in_and_out_moves_one_step() {
        assert_eq!(ZoomLevel::DEFAULT.step(ZoomStep::In), level(115));
        assert_eq!(ZoomLevel::DEFAULT.step(ZoomStep::Out), level(85));
        assert_eq!(level(75).step(ZoomStep::In), level(85));
        assert_eq!(level(250).step(ZoomStep::Out), level(200));
    }

    #[test]
    fn the_ends_stay_where_they_are() {
        assert_eq!(ZoomLevel::MAX.step(ZoomStep::In), ZoomLevel::MAX);
        assert_eq!(ZoomLevel::MIN.step(ZoomStep::Out), ZoomLevel::MIN);
        assert_eq!(ZoomLevel::MAX.percent(), 300);
        assert_eq!(ZoomLevel::MIN.percent(), 50);
    }

    #[test]
    fn one_step_from_an_end_still_moves() {
        assert_eq!(level(250).step(ZoomStep::In), ZoomLevel::MAX);
        assert_eq!(level(75).step(ZoomStep::Out), ZoomLevel::MIN);
    }

    #[test]
    fn reset_goes_back_to_actual_size_from_anywhere() {
        for percent in SAFARI {
            assert_eq!(level(percent).step(ZoomStep::Reset), ZoomLevel::DEFAULT);
        }
    }

    #[test]
    fn only_a_step_is_a_level() {
        for percent in SAFARI {
            assert_eq!(level(percent).percent(), percent);
        }
        for percent in [0, 49, 90, 110, 301] {
            assert_eq!(ZoomLevel::from_percent(percent), None, "{percent}");
        }
    }

    #[test]
    fn a_level_reads_as_a_percentage() {
        assert_eq!(level(115).to_string(), "115 %");
        assert_eq!(ZoomLevel::DEFAULT.to_string(), "100 %");
    }
}

mod sites {
    use super::*;

    #[test]
    fn every_site_starts_at_actual_size() {
        let zooms = SiteZooms::new();

        assert_eq!(zooms.level(&site("example.com")), ZoomLevel::DEFAULT);
        assert_eq!(zooms.iter().count(), 0);
    }

    #[test]
    fn a_site_keeps_its_zoom_and_others_do_not_share_it() {
        let mut zooms = SiteZooms::new();

        assert!(zooms.set(site("example.com"), level(150)));

        assert_eq!(zooms.level(&site("example.com")), level(150));
        assert_eq!(zooms.level(&site("example.org")), ZoomLevel::DEFAULT);
    }

    #[test]
    fn a_site_is_its_page_s_host_without_www() {
        let mut zooms = SiteZooms::new();
        let page = Url::parse("https://www.example.com/a").unwrap();

        zooms.step(Site::try_from(&page).unwrap(), ZoomStep::In);

        assert_eq!(zooms.level(&site("example.com")), level(115));
    }

    #[test]
    fn setting_the_same_zoom_changes_nothing() {
        let mut zooms = SiteZooms::new();
        zooms.set(site("example.com"), level(125));

        assert!(!zooms.set(site("example.com"), level(125)));
        assert!(!zooms.set(site("example.org"), ZoomLevel::DEFAULT));
    }

    #[test]
    fn back_at_actual_size_a_site_leaves_the_list() {
        let mut zooms = SiteZooms::new();
        zooms.set(site("example.com"), level(125));

        assert!(zooms.set(site("example.com"), ZoomLevel::DEFAULT));

        assert_eq!(zooms, SiteZooms::new());
    }

    #[test]
    fn stepping_moves_from_the_site_s_own_zoom() {
        let mut zooms = SiteZooms::new();

        assert_eq!(zooms.step(site("a.test"), ZoomStep::In), level(115));
        assert_eq!(zooms.step(site("a.test"), ZoomStep::In), level(125));
        assert_eq!(zooms.step(site("b.test"), ZoomStep::Out), level(85));
        assert_eq!(zooms.level(&site("a.test")), level(125));
        assert_eq!(
            zooms.step(site("a.test"), ZoomStep::Reset),
            ZoomLevel::DEFAULT
        );
        assert_eq!(
            zooms.iter().collect::<Vec<_>>(),
            [(&site("b.test"), level(85))]
        );
    }

    #[test]
    fn sites_are_listed_in_the_order_first_zoomed() {
        let mut zooms = SiteZooms::new();
        zooms.set(site("b.test"), level(150));
        zooms.set(site("a.test"), level(75));
        zooms.set(site("b.test"), level(175));

        let listed: Vec<_> = zooms.iter().collect();

        assert_eq!(
            listed,
            [(&site("b.test"), level(175)), (&site("a.test"), level(75))]
        );
    }
}

mod snapshots {
    use super::*;

    #[test]
    fn zooms_round_trip_through_a_snapshot() {
        let mut zooms = SiteZooms::new();
        zooms.set(site("b.test"), level(150));
        zooms.set(site("a.test"), level(50));

        let snapshot = zooms.snapshot();

        assert_eq!(
            snapshot.sites,
            [(site("b.test"), 150), (site("a.test"), 50)]
        );
        assert_eq!(SiteZooms::restore(snapshot), Ok(zooms));
    }

    #[test]
    fn an_empty_snapshot_is_no_zoom() {
        assert_eq!(
            SiteZooms::restore(SiteZoomsSnapshot::default()),
            Ok(SiteZooms::new())
        );
    }

    #[test]
    fn a_saved_actual_size_is_left_out() {
        let snapshot = SiteZoomsSnapshot {
            sites: vec![(site("a.test"), 100), (site("b.test"), 125)],
        };

        let zooms = SiteZooms::restore(snapshot).unwrap();

        assert_eq!(
            zooms.iter().collect::<Vec<_>>(),
            [(&site("b.test"), level(125))]
        );
    }

    #[test]
    fn a_site_saved_twice_is_corrupt() {
        let snapshot = SiteZoomsSnapshot {
            sites: vec![(site("a.test"), 150), (site("a.test"), 75)],
        };

        let error = SiteZooms::restore(snapshot).unwrap_err();

        assert_eq!(error, CorruptZooms::DuplicateSite(site("a.test")));
        assert_eq!(error.to_string(), "the site a.test appears twice");
    }

    #[test]
    fn a_zoom_between_steps_is_corrupt() {
        let snapshot = SiteZoomsSnapshot {
            sites: vec![(site("a.test"), 110)],
        };

        let error = SiteZooms::restore(snapshot).unwrap_err();

        assert_eq!(error, CorruptZooms::NotAStep(site("a.test"), 110));
        assert_eq!(
            error.to_string(),
            "a.test is zoomed to 110 %, which is not a zoom step"
        );
    }
}
