//! Floating a video that plays in a frame. The video fills its own frame's viewport, and that
//! frame's `<iframe>` element must fill its parent's, and so on up to the main frame, which
//! fills the page view in the mini player.
//!
//! `WebKit`'s public API names a frame (`WKFrameInfo`) but not its parent, and a parent cannot
//! look into a frame of another origin. So the chain is found one link at a time: the video's
//! frame describes itself (its token, its URL, its viewport's size); every other frame is asked
//! to hold the `<iframe>` that matches the description (its own token when it can see into the
//! frame, else the URL and the size). The frame that holds one describes itself in turn, until
//! the main frame holds one. A round in which no frame holds one ends the chain: the video then
//! fills only its frame, which stays where it is in its page.
//!
//! Pure bookkeeping over the frames' tokens; the page sends each round and hands back the
//! answers.

/// The chain of a page's video being floated.
#[derive(Debug)]
pub(crate) struct Chain {
    /// The frames not in the chain yet: their tokens and whether each is the main frame.
    rest: Vec<(String, bool)>,
    /// The round asked last; answers to an earlier one are stale.
    round: u32,
    /// How many frames have not answered the round yet.
    waiting: usize,
    over: bool,
}

/// One round to send: ask each of `tokens` to hold the frame `description` describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Round {
    pub(crate) number: u32,
    pub(crate) description: String,
    pub(crate) tokens: Vec<String>,
}

impl Chain {
    /// The chain of the video in frame `floated`, among `frames` (each frame's token and
    /// whether it is the main frame).
    pub(crate) fn new(frames: impl IntoIterator<Item = (String, bool)>, floated: &str) -> Self {
        Self {
            rest: frames
                .into_iter()
                .filter(|(token, _)| token != floated)
                .collect(),
            round: 0,
            waiting: 0,
            over: false,
        }
    }

    /// True once the chain reached the main frame or no frame held the last link.
    #[cfg(test)]
    fn is_over(&self) -> bool {
        self.over
    }

    /// The video's frame floated its video and described itself (empty when it had no video
    /// to float). The first round, unless that frame is the main frame.
    pub(crate) fn floated(&mut self, main: bool, description: &str) -> Option<Round> {
        if main || description.is_empty() {
            self.over = true;
            return None;
        }
        self.next(description)
    }

    /// Frame `token` answered round `round`: its description when it held the frame asked
    /// for, empty when it holds none. The next round when the chain goes on.
    pub(crate) fn answered(&mut self, round: u32, token: &str, answer: &str) -> Option<Round> {
        if self.over || round != self.round {
            return None;
        }
        let index = self.rest.iter().position(|(rest, _)| rest == token)?;
        self.waiting = self.waiting.saturating_sub(1);
        if answer.is_empty() {
            self.over = self.waiting == 0;
            return None;
        }
        let (_, main) = self.rest.remove(index);
        if main {
            self.over = true;
            return None;
        }
        self.next(answer)
    }

    fn next(&mut self, description: &str) -> Option<Round> {
        if self.rest.is_empty() {
            self.over = true;
            return None;
        }
        self.round += 1;
        self.waiting = self.rest.len();
        Some(Round {
            number: self.round,
            description: description.to_owned(),
            tokens: self.rest.iter().map(|(token, _)| token.clone()).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Chain, Round};

    /// The main frame `m`, a frame `a` in it, and frames `b` (in `a`) and `c`.
    fn chain(floated: &str) -> Chain {
        let frames = [("m", true), ("a", false), ("b", false), ("c", false)];
        Chain::new(
            frames.map(|(token, main)| (token.to_owned(), main)),
            floated,
        )
    }

    /// Round `number`, asking `tokens` for the frame `description` describes.
    fn round(number: u32, description: &str, tokens: &[&str]) -> Round {
        Round {
            number,
            description: description.to_owned(),
            tokens: tokens.iter().map(|&token| token.to_owned()).collect(),
        }
    }

    #[test]
    fn a_video_in_the_main_frame_needs_no_chain() {
        let mut chain = chain("m");
        assert_eq!(chain.floated(true, "{m}"), None);
        assert!(chain.is_over());
    }

    #[test]
    fn a_frame_with_nothing_to_float_ends_the_chain() {
        let mut chain = chain("b");
        assert_eq!(chain.floated(false, ""), None);
        assert!(chain.is_over());
    }

    #[test]
    fn every_other_frame_is_asked_for_the_videos_frame() {
        let mut chain = chain("b");
        assert_eq!(
            chain.floated(false, "{b}"),
            Some(round(1, "{b}", &["m", "a", "c"]))
        );
        assert!(!chain.is_over());
    }

    #[test]
    fn each_link_asks_for_the_next_until_the_main_frame_holds_one() {
        let mut chain = chain("b");
        chain.floated(false, "{b}");
        assert_eq!(chain.answered(1, "m", ""), None);
        assert_eq!(
            chain.answered(1, "a", "{a}"),
            Some(round(2, "{a}", &["m", "c"]))
        );
        assert_eq!(chain.answered(1, "c", ""), None, "a stale answer");
        assert!(!chain.is_over());
        assert_eq!(chain.answered(2, "m", "{m}"), None);
        assert!(chain.is_over());
    }

    #[test]
    fn a_round_no_frame_holds_ends_the_chain() {
        let mut chain = chain("b");
        chain.floated(false, "{b}");
        assert_eq!(chain.answered(1, "m", ""), None);
        assert_eq!(chain.answered(1, "a", ""), None);
        assert!(!chain.is_over());
        assert_eq!(chain.answered(1, "c", ""), None);
        assert!(chain.is_over());
    }

    #[test]
    fn nothing_is_asked_once_the_chain_is_over() {
        let mut chain = chain("a");
        chain.floated(false, "{a}");
        chain.answered(1, "m", "{m}");
        assert!(chain.is_over());
        assert_eq!(chain.answered(1, "b", "{b}"), None);
    }

    #[test]
    fn an_answer_from_a_frame_not_asked_is_ignored() {
        let mut chain = chain("a");
        chain.floated(false, "{a}");
        assert_eq!(chain.answered(1, "a", "{a}"), None);
        assert_eq!(chain.answered(1, "z", "{z}"), None);
        assert!(!chain.is_over());
    }

    #[test]
    fn with_no_other_frame_left_the_chain_ends() {
        let mut chain = Chain::new([("a".to_owned(), false)], "a");
        assert_eq!(chain.floated(false, "{a}"), None);
        assert!(chain.is_over());
    }
}
