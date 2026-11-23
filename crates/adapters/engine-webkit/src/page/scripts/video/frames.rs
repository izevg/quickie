//! Which frame of a page shows its video. The video script runs in every frame, the main
//! frame and each `<iframe>` (a video site's embed on a blog, an ad, a player inside a
//! player), and each instance reports for its own frame under a random token. A page's video
//! prefers a frame playing with sound, then the one that started playing last. This keeps each
//! frame's last report and says when the page's video changes.
//!
//! Pure bookkeeping, generic over what names a frame to `WebKit` (its `WKFrameInfo`), so it is
//! tested without a page.

use quickie_domain_browsing::Video;

use super::Report;

/// What a frame's video script said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Said {
    /// What the frame's video does now, `None` while it has none.
    Video(Option<Video>),
    /// The frame's document is going away (`pagehide`): forget the frame.
    Left,
}

/// One report of the video script: what it said, and which frame said it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameReport<'a> {
    pub(crate) said: Said,
    pub(crate) token: &'a str,
}

impl<'a> FrameReport<'a> {
    /// Reads one report: `A` (plays aloud), `M` (plays muted), `P` (paused), `N` (no video),
    /// or `X` (leaving), then the frame's token. `None` for a report that is not the script's.
    pub(crate) fn parse(text: &'a str) -> Option<Self> {
        let mut chars = text.chars();
        let said = match chars.next()? {
            'A' => Said::Video(Some(Video::Playing { audible: true })),
            'M' => Said::Video(Some(Video::Playing { audible: false })),
            'P' => Said::Video(Some(Video::Paused)),
            'N' => Said::Video(None),
            'X' => Said::Left,
            _ => return None,
        };
        let token = chars.as_str();
        let valid = !token.is_empty() && token.chars().all(|c| c.is_ascii_alphanumeric());
        valid.then_some(Self { said, token })
    }
}

/// One frame of a page, as its script last reported.
#[derive(Clone, Debug)]
pub(crate) struct Frame<F> {
    pub(crate) token: String,
    /// What names the frame to `WebKit`; `None` for the main frame.
    pub(crate) handle: Option<F>,
    video: Option<Video>,
    /// When its video last started playing, on the page's own clock; 0 if it never did.
    played: u64,
}

/// The frames of one page and the video the page shows.
#[derive(Clone, Debug)]
pub(crate) struct Frames<F> {
    known: Vec<Frame<F>>,
    clock: u64,
    /// The frame that owns the mini player's controls until the video docks.
    floated: Option<String>,
    /// The page's video as last told: a report changes it or not.
    told: Option<Video>,
}

impl<F> Default for Frames<F> {
    fn default() -> Self {
        Self {
            known: Vec::new(),
            clock: 0,
            floated: None,
            told: None,
        }
    }
}

impl<F> Frames<F> {
    /// Applies what the frame `token` said; `handle` names it, `None` for the main frame.
    /// The page's video when that changed, `None` when it did not.
    ///
    /// A main frame not heard before starts a new document: the frames of the old one are
    /// forgotten even if their leaving was never heard.
    pub(crate) fn heard(&mut self, token: &str, handle: Option<F>, said: Said) -> Option<Report> {
        let index = self.known.iter().position(|frame| frame.token == token);
        match (said, index) {
            (Said::Left, Some(index)) => {
                self.known.remove(index);
            }
            (Said::Left, None) => {}
            (Said::Video(video), Some(index)) => {
                let frame = &mut self.known[index];
                if playing(video) && !playing(frame.video) {
                    self.clock += 1;
                    frame.played = self.clock;
                }
                frame.video = video;
            }
            (Said::Video(video), None) => {
                if handle.is_none() {
                    self.known.clear();
                }
                let played = if playing(video) {
                    self.clock += 1;
                    self.clock
                } else {
                    0
                };
                self.known.push(Frame {
                    token: token.to_owned(),
                    handle,
                    video,
                    played,
                });
            }
        }
        self.report_if_changed()
    }

    fn report_if_changed(&mut self) -> Option<Report> {
        let now = self.showing().and_then(|frame| frame.video);
        (now != self.told).then(|| {
            self.told = now;
            Report(now)
        })
    }

    /// Keeps reports and controls on the frame whose video is being floated.
    pub(crate) fn pin(&mut self) {
        self.floated = self.showing().map(|frame| frame.token.clone());
    }

    /// Selects normally again after docking, reporting a different video state if needed.
    pub(crate) fn unpin(&mut self) -> Option<Report> {
        self.floated = None;
        self.report_if_changed()
    }

    /// The floated frame, or the newest frame playing with sound. Without either, the frame
    /// whose video started playing last stays selected.
    pub(crate) fn showing(&self) -> Option<&Frame<F>> {
        if let Some(floated) = &self.floated {
            return self
                .known
                .iter()
                .find(|frame| &frame.token == floated && frame.video.is_some());
        }
        self.known
            .iter()
            .filter(|frame| frame.video.is_some())
            .max_by_key(|frame| {
                (
                    matches!(frame.video, Some(Video::Playing { audible: true })),
                    frame.played,
                )
            })
    }

    /// Every frame heard from and not gone, in the order they were first heard.
    pub(crate) fn all(&self) -> &[Frame<F>] {
        &self.known
    }
}

fn playing(video: Option<Video>) -> bool {
    matches!(video, Some(Video::Playing { .. }))
}

#[cfg(test)]
mod tests {
    use quickie_domain_browsing::Video;

    use super::{FrameReport, Frames, Report, Said};

    const ALOUD: Option<Video> = Some(Video::Playing { audible: true });
    const MUTED: Option<Video> = Some(Video::Playing { audible: false });
    const PAUSED: Option<Video> = Some(Video::Paused);

    /// The page's main frame, with nothing playing, and frames `a` and `b` in it.
    fn page() -> Frames<char> {
        let mut frames = Frames::default();
        frames.heard("main", None, Said::Video(None));
        frames.heard("a", Some('a'), Said::Video(None));
        frames.heard("b", Some('b'), Said::Video(None));
        frames
    }

    /// Frame `token` (the main frame when `main`) says `video`; what the page then says.
    fn says(frames: &mut Frames<char>, token: &str, video: Option<Video>) -> Option<Report> {
        let handle = (token != "main").then(|| token.chars().next().unwrap_or('?'));
        frames.heard(token, handle, Said::Video(video))
    }

    fn leaves(frames: &mut Frames<char>, token: &str) -> Option<Report> {
        let handle = token.chars().next();
        frames.heard(token, handle, Said::Left)
    }

    fn showing(frames: &Frames<char>) -> Option<&str> {
        frames.showing().map(|frame| frame.token.as_str())
    }

    fn tokens(frames: &Frames<char>) -> Vec<&str> {
        frames
            .all()
            .iter()
            .map(|frame| frame.token.as_str())
            .collect()
    }

    #[test]
    fn each_report_says_what_a_frame_does_and_who_it_is() {
        let report = |said| Some(FrameReport { said, token: "k3x" });
        assert_eq!(FrameReport::parse("Ak3x"), report(Said::Video(ALOUD)));
        assert_eq!(FrameReport::parse("Mk3x"), report(Said::Video(MUTED)));
        assert_eq!(FrameReport::parse("Pk3x"), report(Said::Video(PAUSED)));
        assert_eq!(FrameReport::parse("Nk3x"), report(Said::Video(None)));
        assert_eq!(FrameReport::parse("Xk3x"), report(Said::Left));
    }

    #[test]
    fn a_report_not_from_the_script_is_dropped() {
        for junk in ["", "A", "N", "a1", "Z1", " A1", "A 1", "A1;"] {
            assert_eq!(FrameReport::parse(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn a_video_in_a_frame_is_the_pages_video() {
        let mut frames = page();
        assert_eq!(says(&mut frames, "a", ALOUD), Some(Report(ALOUD)));
        assert_eq!(showing(&frames), Some("a"));
        assert_eq!(frames.showing().and_then(|frame| frame.handle), Some('a'));
    }

    #[test]
    fn a_frame_first_heard_playing_shows_its_video() {
        let mut frames = page();
        assert_eq!(says(&mut frames, "c", MUTED), Some(Report(MUTED)));
        assert_eq!(showing(&frames), Some("c"));
    }

    #[test]
    fn the_audible_video_that_started_last_wins_in_any_frame() {
        let mut frames = page();
        says(&mut frames, "main", ALOUD);

        assert_eq!(says(&mut frames, "b", ALOUD), None);
        assert_eq!(showing(&frames), Some("b"));
    }

    #[test]
    fn a_later_muted_frame_does_not_replace_an_audible_frame() {
        let mut frames = page();
        says(&mut frames, "a", ALOUD);

        assert_eq!(says(&mut frames, "b", MUTED), None);
        assert_eq!(showing(&frames), Some("a"));
    }

    #[test]
    fn when_the_audible_frame_mutes_the_newest_muted_frame_is_selected() {
        let mut frames = page();
        says(&mut frames, "a", ALOUD);
        says(&mut frames, "b", MUTED);

        assert_eq!(says(&mut frames, "a", MUTED), Some(Report(MUTED)));
        assert_eq!(showing(&frames), Some("b"));
    }

    #[test]
    fn the_floated_frame_stays_selected_when_paused_and_resumed() {
        let mut frames = page();
        says(&mut frames, "a", ALOUD);
        frames.pin();
        says(&mut frames, "a", PAUSED);

        assert_eq!(says(&mut frames, "b", ALOUD), None);
        assert_eq!(showing(&frames), Some("a"));
        assert_eq!(says(&mut frames, "a", ALOUD), Some(Report(ALOUD)));
    }

    #[test]
    fn docking_reports_the_reselected_video_state() {
        let mut frames = page();
        says(&mut frames, "a", ALOUD);
        frames.pin();
        says(&mut frames, "a", PAUSED);
        says(&mut frames, "b", ALOUD);

        assert_eq!(frames.unpin(), Some(Report(ALOUD)));
        assert_eq!(showing(&frames), Some("b"));
    }

    #[test]
    fn a_floated_frame_leaving_reports_its_video_gone() {
        let mut frames = page();
        says(&mut frames, "a", ALOUD);
        frames.pin();
        says(&mut frames, "b", ALOUD);

        assert_eq!(leaves(&mut frames, "a"), Some(Report(None)));
        assert_eq!(showing(&frames), None);
    }

    #[test]
    fn a_paused_video_stays_the_pages_until_another_plays() {
        let mut frames = page();
        says(&mut frames, "a", ALOUD);
        assert_eq!(says(&mut frames, "a", PAUSED), Some(Report(PAUSED)));
        assert_eq!(showing(&frames), Some("a"));
        assert_eq!(says(&mut frames, "a", ALOUD), Some(Report(ALOUD)));
    }

    #[test]
    fn a_video_played_again_comes_before_one_that_played_since() {
        let mut frames = page();
        says(&mut frames, "a", ALOUD);
        says(&mut frames, "b", MUTED);
        says(&mut frames, "a", PAUSED);
        assert_eq!(says(&mut frames, "a", ALOUD), Some(Report(ALOUD)));
        assert_eq!(showing(&frames), Some("a"));
    }

    #[test]
    fn when_the_frame_leaves_the_video_before_comes_back() {
        let mut frames = page();
        says(&mut frames, "a", MUTED);
        says(&mut frames, "b", ALOUD);
        assert_eq!(leaves(&mut frames, "b"), Some(Report(MUTED)));
        assert_eq!(showing(&frames), Some("a"));
        assert_eq!(leaves(&mut frames, "a"), Some(Report(None)));
        assert_eq!(showing(&frames), None);
    }

    #[test]
    fn nothing_changes_while_no_video_plays() {
        let mut frames = page();
        assert_eq!(says(&mut frames, "c", None), None);
        assert_eq!(leaves(&mut frames, "a"), None);
        assert_eq!(leaves(&mut frames, "gone"), None);
        assert_eq!(tokens(&frames), ["main", "b", "c"]);
    }

    #[test]
    fn a_new_main_document_forgets_the_old_frames() {
        let mut frames = page();
        says(&mut frames, "a", ALOUD);
        assert_eq!(
            frames.heard("next", None, Said::Video(None)),
            Some(Report(None))
        );
        assert_eq!(tokens(&frames), ["next"]);
        assert!(frames.all()[0].handle.is_none());
    }

    #[test]
    fn the_main_frame_saying_more_keeps_its_frames() {
        let mut frames = page();
        says(&mut frames, "main", MUTED);
        assert_eq!(tokens(&frames), ["main", "a", "b"]);
    }
}
