//! A tab's video and the mini player: when the user leaves a tab whose video plays with its
//! sound on, that video floats over the window in a mini player, until the user comes back to
//! the tab, closes the player, closes the tab, or leaves another tab whose video plays, which
//! takes its place. It shows only because the user left a tab, never on its own.

use std::collections::BTreeMap;

use quickie_domain_kernel::TabId;

/// What the video a tab's page plays last is doing, as the page reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Video {
    /// It plays; `audible` when its sound is on (not muted, the volume above zero). A muted
    /// video that plays by itself, a page's moving backdrop, is not what the user watches.
    Playing { audible: bool },
    /// It is paused, or played to its end.
    Paused,
}

/// What a step changed: whose video the mini player stops showing, and whose it shows now.
/// A second video replacing the first ends one and shows the other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerChange {
    /// The tab whose video the player no longer shows: its page shows whole again.
    pub ended: Option<TabId>,
    /// The tab whose video the player shows from now on.
    pub shown: Option<TabId>,
}

/// The videos of the tabs and the one tab whose video the mini player shows, if any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MiniPlayer {
    videos: BTreeMap<TabId, Video>,
    shown: Option<TabId>,
}

impl MiniPlayer {
    /// No video known, nothing shown.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The tab whose video the player shows.
    #[must_use]
    pub fn shown(&self) -> Option<TabId> {
        self.shown
    }

    /// What the tab's video does, as its page last said; `None` when it has none.
    #[must_use]
    pub fn video(&self, tab: TabId) -> Option<Video> {
        self.videos.get(&tab).copied()
    }

    /// The tab's page reported its video, `None` when it has none any more (it left the page,
    /// or the video went away). The player ends when the video it shows is gone; a pause
    /// keeps it, so that it can play on.
    pub fn reported(&mut self, tab: TabId, video: Option<Video>) -> PlayerChange {
        if let Some(video) = video {
            self.videos.insert(tab, video);
            PlayerChange::default()
        } else {
            self.videos.remove(&tab);
            self.end_if_shown(tab)
        }
    }

    /// The user left tab `from` for tab `to` in a window. Coming back to the tab the player
    /// shows ends it; leaving a tab whose video plays with its sound on shows that video,
    /// in place of any other.
    pub fn switched(&mut self, from: TabId, to: TabId) -> PlayerChange {
        if from == to {
            return PlayerChange::default();
        }
        let mut change = self.end_if_shown(to);
        let watched = self.video(from) == Some(Video::Playing { audible: true });
        if watched && self.shown != Some(from) {
            if let Some(replaced) = self.shown.replace(from) {
                change.ended = Some(replaced);
            }
            change.shown = Some(from);
        }
        change
    }

    /// The tab closed: its video is forgotten, and the player ends if it showed it.
    pub fn closed(&mut self, tab: TabId) -> PlayerChange {
        self.videos.remove(&tab);
        self.end_if_shown(tab)
    }

    /// The user closed the player.
    pub fn dismissed(&mut self) -> PlayerChange {
        PlayerChange {
            ended: self.shown.take(),
            shown: None,
        }
    }

    fn end_if_shown(&mut self, tab: TabId) -> PlayerChange {
        if self.shown == Some(tab) {
            self.dismissed()
        } else {
            PlayerChange::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use quickie_domain_kernel::{Profile, TabId};

    use super::{MiniPlayer, PlayerChange, Video};

    const WATCHED: Option<Video> = Some(Video::Playing { audible: true });

    fn tab(serial: u64) -> TabId {
        TabId::new(Profile::Main, serial)
    }

    fn shows(tab: TabId) -> PlayerChange {
        PlayerChange {
            ended: None,
            shown: Some(tab),
        }
    }

    fn ends(tab: TabId) -> PlayerChange {
        PlayerChange {
            ended: Some(tab),
            shown: None,
        }
    }

    /// A player that shows tab 1's video, left for tab 2.
    fn showing_one() -> MiniPlayer {
        let mut player = MiniPlayer::new();
        player.reported(tab(1), WATCHED);
        assert_eq!(player.switched(tab(1), tab(2)), shows(tab(1)));
        player
    }

    #[test]
    fn leaving_a_tab_whose_video_plays_aloud_shows_it() {
        let player = showing_one();

        assert_eq!(player.shown(), Some(tab(1)));
        assert_eq!(player.video(tab(1)), WATCHED);
    }

    #[test]
    fn a_paused_muted_or_absent_video_shows_nothing() {
        for video in [
            None,
            Some(Video::Paused),
            Some(Video::Playing { audible: false }),
        ] {
            let mut player = MiniPlayer::new();
            player.reported(tab(1), video);

            assert_eq!(player.switched(tab(1), tab(2)), PlayerChange::default());
            assert_eq!(player.shown(), None, "{video:?}");
        }
    }

    #[test]
    fn staying_on_the_same_tab_is_no_switch() {
        let mut player = MiniPlayer::new();
        player.reported(tab(1), WATCHED);

        assert_eq!(player.switched(tab(1), tab(1)), PlayerChange::default());
        assert_eq!(player.shown(), None);
    }

    #[test]
    fn coming_back_to_the_tab_ends_the_player() {
        let mut player = showing_one();

        assert_eq!(player.switched(tab(2), tab(1)), ends(tab(1)));
        assert_eq!(player.shown(), None);
    }

    #[test]
    fn going_to_another_tab_keeps_the_player() {
        let mut player = showing_one();

        assert_eq!(player.switched(tab(2), tab(3)), PlayerChange::default());
        assert_eq!(player.shown(), Some(tab(1)));
    }

    /// The shown tab, active in another window, left again: it already floats.
    #[test]
    fn leaving_the_shown_tab_again_changes_nothing() {
        let mut player = showing_one();

        assert_eq!(player.switched(tab(1), tab(3)), PlayerChange::default());
        assert_eq!(player.shown(), Some(tab(1)));
    }

    #[test]
    fn a_second_playing_video_replaces_the_first() {
        let mut player = showing_one();
        player.reported(tab(2), WATCHED);

        assert_eq!(
            player.switched(tab(2), tab(3)),
            PlayerChange {
                ended: Some(tab(1)),
                shown: Some(tab(2)),
            }
        );
        assert_eq!(player.shown(), Some(tab(2)));
    }

    #[test]
    fn leaving_a_playing_tab_for_the_shown_one_swaps_them() {
        let mut player = showing_one();
        player.reported(tab(2), WATCHED);

        assert_eq!(
            player.switched(tab(2), tab(1)),
            PlayerChange {
                ended: Some(tab(1)),
                shown: Some(tab(2)),
            }
        );
        assert_eq!(player.shown(), Some(tab(2)));
    }

    #[test]
    fn a_pause_keeps_the_player_and_a_gone_video_ends_it() {
        let mut player = showing_one();

        assert_eq!(
            player.reported(tab(1), Some(Video::Paused)),
            PlayerChange::default()
        );
        assert_eq!(player.shown(), Some(tab(1)));
        assert_eq!(player.video(tab(1)), Some(Video::Paused));

        assert_eq!(player.reported(tab(1), None), ends(tab(1)));
        assert_eq!(player.shown(), None);
        assert_eq!(player.video(tab(1)), None);
    }

    #[test]
    fn another_tab_s_video_going_away_keeps_the_player() {
        let mut player = showing_one();
        player.reported(tab(2), WATCHED);

        assert_eq!(player.reported(tab(2), None), PlayerChange::default());
        assert_eq!(player.shown(), Some(tab(1)));
    }

    #[test]
    fn closing_the_shown_tab_ends_the_player_and_forgets_its_video() {
        let mut player = showing_one();

        assert_eq!(player.closed(tab(1)), ends(tab(1)));
        assert_eq!(player.shown(), None);
        assert_eq!(player.video(tab(1)), None);
    }

    #[test]
    fn closing_another_tab_keeps_the_player() {
        let mut player = showing_one();
        player.reported(tab(3), WATCHED);

        assert_eq!(player.closed(tab(3)), PlayerChange::default());
        assert_eq!(player.shown(), Some(tab(1)));
        assert_eq!(player.video(tab(3)), None);
    }

    #[test]
    fn closing_the_player_ends_it_once() {
        let mut player = showing_one();

        assert_eq!(player.dismissed(), ends(tab(1)));
        assert_eq!(player.dismissed(), PlayerChange::default());
        assert_eq!(player.switched(tab(2), tab(1)), PlayerChange::default());
    }
}
