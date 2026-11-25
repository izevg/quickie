//! The mini player: the video of a tab the user left floats over the window while it plays,
//! as the [`MiniPlayer`] of the browsing context decides, and the engine moves the page there
//! and back ([`PageVideo`]). The shell routes what pages report and what the user does, and
//! puts the player on screen as each step's [`PlayerChange`] says.

use quickie_domain_browsing::{MiniPlayer, PlayerChange, TabId, Video};

use crate::{AppError, PageVideo};

/// The mini player of one profile.
#[derive(Debug, Default)]
pub struct Playback {
    player: MiniPlayer,
}

impl Playback {
    /// No video known, nothing floating.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The tab whose video floats.
    #[must_use]
    pub fn shown(&self) -> Option<TabId> {
        self.player.shown()
    }

    /// True while the video that floats plays.
    #[must_use]
    pub fn playing(&self) -> bool {
        self.shown()
            .and_then(|tab| self.player.video(tab))
            .is_some_and(|video| matches!(video, Video::Playing { .. }))
    }

    /// A page reported what its video does ([`crate::EngineEvent::Video`]); the floating page
    /// docks when its video is gone.
    pub fn reported<E: PageVideo + ?Sized>(
        &mut self,
        engine: &mut E,
        tab: TabId,
        video: Option<Video>,
    ) -> PlayerChange {
        let change = self.player.reported(tab, video);
        dock(engine, change);
        change
    }

    /// The user left tab `from` for tab `to`: the tab come back to docks, and the video of the
    /// tab left floats into `slot` when it plays aloud, in place of any other.
    ///
    /// # Errors
    ///
    /// The engine could not float the video; nothing floats then.
    pub fn switched<E: PageVideo + ?Sized>(
        &mut self,
        engine: &mut E,
        slot: &E::View,
        from: TabId,
        to: TabId,
    ) -> Result<PlayerChange, AppError> {
        let change = self.player.switched(from, to);
        dock(engine, change);
        if let Some(tab) = change.shown
            && let Err(error) = engine.float_video(tab, slot)
        {
            self.player.dismissed();
            return Err(error.into());
        }
        Ok(change)
    }

    /// The tab closed, and its page with it: the player ends if it showed it.
    pub fn closed(&mut self, tab: TabId) -> PlayerChange {
        self.player.closed(tab)
    }

    /// Docks the floating page while preserving its playback and the active tab.
    pub fn minimize<E: PageVideo + ?Sized>(&mut self, engine: &mut E) -> PlayerChange {
        let change = self.player.dismissed();
        dock(engine, change);
        change
    }

    /// The user closed the player: its video pauses, and its page shows whole again.
    ///
    /// # Errors
    ///
    /// The engine could not pause the video; the player ends all the same.
    pub fn dismiss<E: PageVideo + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<PlayerChange, AppError> {
        let paused = self.shown().map(|tab| engine.play_video(tab, false));
        let change = self.minimize(engine);
        paused.transpose()?;
        Ok(change)
    }

    /// The user pressed play or pause: the floating video pauses when it plays, and plays when
    /// it does not. The page reports the change.
    ///
    /// # Errors
    ///
    /// The engine could not play or pause the video.
    pub fn toggle<E: PageVideo + ?Sized>(&mut self, engine: &mut E) -> Result<(), AppError> {
        if let Some(tab) = self.shown() {
            engine.play_video(tab, !self.playing())?;
        }
        Ok(())
    }
}

/// The page whose video the player no longer shows docks.
fn dock<E: PageVideo + ?Sized>(engine: &mut E, change: PlayerChange) {
    if let Some(tab) = change.ended {
        engine.dock_video(tab);
    }
}

#[cfg(test)]
mod tests {
    use quickie_domain_browsing::{Jar, PlayerChange, Profile, TabId, Video};
    use url::Url;

    use super::Playback;
    use crate::conformance::{MemoryEngine, MemoryView};
    use crate::{Engine, EngineEvent, EnginePort};

    const WATCHED: Option<Video> = Some(Video::Playing { audible: true });

    fn tab(serial: u64) -> TabId {
        TabId::new(Profile::Main, serial)
    }

    /// An engine with pages for tabs 1 to 3.
    fn engine() -> MemoryEngine {
        let base = Url::parse("http://playback.test/").unwrap();
        let mut engine = MemoryEngine::new(base.clone());
        for serial in 1..=3 {
            engine.create(tab(serial), &base, Jar::Main).unwrap();
        }
        engine
    }

    /// Tab 1's video plays aloud and the user left it for tab 2.
    fn floating() -> (Playback, MemoryEngine) {
        let mut engine = engine();
        let mut playback = Playback::new();
        engine.play(tab(1), Video::Playing { audible: true });
        playback.reported(&mut engine, tab(1), WATCHED);
        let change = playback
            .switched(&mut engine, &MemoryView, tab(1), tab(2))
            .unwrap();
        assert_eq!(
            change,
            PlayerChange {
                ended: None,
                shown: Some(tab(1)),
            }
        );
        (playback, engine)
    }

    /// The videos the engine reported for `tab` since the last call.
    fn reports(engine: &mut MemoryEngine, about: TabId) -> Vec<Option<Video>> {
        let mut events = Vec::new();
        engine.drain(&mut events);
        events
            .into_iter()
            .filter_map(|event| match event {
                EngineEvent::Video { tab, video } if tab == about => Some(video),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn leaving_a_tab_whose_video_plays_floats_it() {
        let (playback, engine) = floating();

        assert_eq!(engine.floated(), Some(tab(1)));
        assert_eq!(playback.shown(), Some(tab(1)));
        assert!(playback.playing());
    }

    #[test]
    fn coming_back_to_the_tab_docks_it() {
        let (mut playback, mut engine) = floating();

        let change = playback
            .switched(&mut engine, &MemoryView, tab(2), tab(1))
            .unwrap();

        assert_eq!(change.ended, Some(tab(1)));
        assert_eq!(engine.floated(), None);
        assert_eq!(playback.shown(), None);
        assert!(!playback.playing());
    }

    #[test]
    fn a_second_video_takes_the_first_one_s_place() {
        let (mut playback, mut engine) = floating();
        playback.reported(&mut engine, tab(2), WATCHED);

        let change = playback
            .switched(&mut engine, &MemoryView, tab(2), tab(3))
            .unwrap();

        assert_eq!(
            change,
            PlayerChange {
                ended: Some(tab(1)),
                shown: Some(tab(2)),
            }
        );
        assert_eq!(engine.floated(), Some(tab(2)));
    }

    #[test]
    fn a_video_gone_from_its_page_docks_it() {
        let (mut playback, mut engine) = floating();

        let change = playback.reported(&mut engine, tab(1), None);

        assert_eq!(change.ended, Some(tab(1)));
        assert_eq!(engine.floated(), None);
    }

    #[test]
    fn a_video_that_cannot_float_leaves_nothing_floating() {
        let mut engine = engine();
        let mut playback = Playback::new();
        playback.reported(&mut engine, tab(9), WATCHED);

        let floated = playback.switched(&mut engine, &MemoryView, tab(9), tab(2));

        assert!(floated.is_err());
        assert_eq!(playback.shown(), None);
        assert_eq!(engine.floated(), None);
    }

    #[test]
    fn closing_the_floating_tab_ends_the_player() {
        let (mut playback, _engine) = floating();

        assert_eq!(playback.closed(tab(1)).ended, Some(tab(1)));
        assert_eq!(playback.shown(), None);
    }

    #[test]
    fn closing_the_player_pauses_the_video_and_docks_it() {
        let (mut playback, mut engine) = floating();
        reports(&mut engine, tab(1));

        let change = playback.dismiss(&mut engine).unwrap();

        assert_eq!(change.ended, Some(tab(1)));
        assert_eq!(reports(&mut engine, tab(1)), [Some(Video::Paused)]);
        assert_eq!(engine.floated(), None);
        assert_eq!(playback.shown(), None);
    }

    #[test]
    fn minimizing_the_player_docks_it_and_preserves_playback() {
        let (mut playback, mut engine) = floating();
        reports(&mut engine, tab(1));

        let change = playback.minimize(&mut engine);

        assert_eq!(change.ended, Some(tab(1)));
        assert_eq!(engine.floated(), None);
        assert_eq!(playback.shown(), None);
        assert_eq!(reports(&mut engine, tab(1)), []);
        assert_eq!(
            playback
                .switched(&mut engine, &MemoryView, tab(1), tab(2))
                .unwrap()
                .shown,
            Some(tab(1))
        );
    }

    #[test]
    fn closing_no_player_pauses_nothing() {
        let mut engine = engine();
        engine.play(tab(1), Video::Playing { audible: true });
        reports(&mut engine, tab(1));

        let change = Playback::new().dismiss(&mut engine).unwrap();

        assert_eq!(change, PlayerChange::default());
        assert_eq!(reports(&mut engine, tab(1)), Vec::new());
    }

    #[test]
    fn a_player_whose_page_is_gone_ends_when_closed_and_says_so() {
        let (mut playback, mut engine) = floating();
        engine.close(tab(1)).unwrap();

        assert!(playback.dismiss(&mut engine).is_err());
        assert_eq!(playback.shown(), None);
    }

    #[test]
    fn play_and_pause_follow_what_the_page_said_last() {
        let (mut playback, mut engine) = floating();
        reports(&mut engine, tab(1));

        playback.toggle(&mut engine).unwrap();
        let paused = reports(&mut engine, tab(1));
        assert_eq!(paused, [Some(Video::Paused)]);

        playback.reported(&mut engine, tab(1), Some(Video::Paused));
        assert!(!playback.playing());
        assert_eq!(playback.shown(), Some(tab(1)));
        playback.toggle(&mut engine).unwrap();

        assert_eq!(reports(&mut engine, tab(1)), [WATCHED]);
    }

    #[test]
    fn play_or_pause_without_a_player_does_nothing() {
        let mut engine = engine();
        engine.play(tab(1), Video::Playing { audible: true });
        reports(&mut engine, tab(1));

        Playback::new().toggle(&mut engine).unwrap();

        assert_eq!(reports(&mut engine, tab(1)), Vec::new());
    }

    #[test]
    fn a_player_whose_page_is_gone_cannot_play() {
        let (mut playback, mut engine) = floating();
        engine.close(tab(1)).unwrap();

        assert!(playback.toggle(&mut engine).is_err());
    }
}
