//! What a download row says, from a [`Download`] and its measured rate. Pure, so every
//! wording is tested without a view.

use quickie_domain_library::{Download, DownloadId, DownloadState};
use quickie_ui_shared::format;

/// Where a download stands, as the row draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// What a row's button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Cancel,
    Reveal,
    Open,
    Retry,
}

impl Phase {
    /// The row's buttons, leading first.
    pub(crate) fn actions(self) -> &'static [Action] {
        match self {
            Self::Running => &[Action::Cancel],
            Self::Done => &[Action::Open, Action::Reveal],
            Self::Failed | Self::Cancelled => &[Action::Retry],
        }
    }
}

/// What `⏎` does to a row in `phase`, or `⌘⏎` when `reveal`: a finished file opens or shows in
/// the Finder, an ended download is retried, and a running one is left alone; cancelling takes
/// a click.
pub(crate) fn on_enter(phase: Phase, reveal: bool) -> Option<Action> {
    match (phase, reveal) {
        (Phase::Done, false) => Some(Action::Open),
        (Phase::Done, true) => Some(Action::Reveal),
        (Phase::Failed | Phase::Cancelled, false) => Some(Action::Retry),
        (Phase::Failed | Phase::Cancelled, true) | (Phase::Running, _) => None,
    }
}

/// One row's content.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub(crate) id: DownloadId,
    pub(crate) name: String,
    pub(crate) detail: String,
    /// How much arrived, `0.0 ..= 1.0`, while the size is known.
    pub(crate) fraction: Option<f64>,
    pub(crate) phase: Phase,
}

/// The row of `download`; `rate` is its measured speed in bytes per second, if there is one.
pub(crate) fn line(download: &Download, rate: Option<u64>) -> Line {
    let host = download.url.host_str().unwrap_or_default();
    let (phase, detail, fraction) = match &download.state {
        DownloadState::InProgress { received, total } => (
            Phase::Running,
            running(*received, *total, rate),
            total.filter(|&total| total > 0).map(|total| {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a bar's fill; the precision lost is far below a pixel"
                )]
                let fraction = *received as f64 / total as f64;
                fraction.clamp(0.0, 1.0)
            }),
        ),
        DownloadState::Finished => (Phase::Done, with_host("Done", host), None),
        DownloadState::Failed(reason) => (Phase::Failed, format!("Failed · {reason}"), None),
        DownloadState::Cancelled => (Phase::Cancelled, with_host("Cancelled", host), None),
    };
    Line {
        id: download.id,
        name: download.file_name.clone(),
        detail,
        fraction,
        phase,
    }
}

/// `184.2 MB of 240 MB · 4.2 MB/s · 12 s left`, with only the parts that are known.
fn running(received: u64, total: Option<u64>, rate: Option<u64>) -> String {
    if received == 0 && rate.is_none() {
        return "Starting…".to_owned();
    }
    let mut parts = vec![match total {
        Some(total) => format!("{} of {}", format::bytes(received), format::bytes(total)),
        None => format::bytes(received),
    }];
    if let Some(rate) = rate.filter(|&rate| rate > 0) {
        parts.push(format::rate(rate));
        if let Some(total) = total {
            parts.push(format::time_left(
                total.saturating_sub(received).div_ceil(rate),
            ));
        }
    }
    parts.join(" · ")
}

fn with_host(state: &str, host: &str) -> String {
    if host.is_empty() {
        state.to_owned()
    } else {
        format!("{state} · {host}")
    }
}

/// A download's speed, smoothed over the samples the popover takes while it shows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RateMeter {
    /// The last sample: bytes received, at seconds since any fixed point.
    last: Option<(u64, f64)>,
    /// Bytes per second, smoothed.
    rate: Option<f64>,
}

impl RateMeter {
    /// How much a new measurement counts against the smoothed rate.
    const WEIGHT: f64 = 0.3;
    /// Samples closer together than this measure noise, not speed.
    const MIN_INTERVAL: f64 = 0.2;

    /// Takes a sample of `received` bytes at `seconds`; returns the rate so far, if any.
    pub(crate) fn sample(&mut self, received: u64, seconds: f64) -> Option<u64> {
        match self.last {
            None => self.last = Some((received, seconds)),
            Some((before, then)) if seconds - then >= Self::MIN_INTERVAL => {
                #[expect(clippy::cast_precision_loss, reason = "a speed shown with one decimal")]
                let measured = received.saturating_sub(before) as f64 / (seconds - then);
                self.rate = Some(match self.rate {
                    None => measured,
                    Some(rate) => rate + (measured - rate) * Self::WEIGHT,
                });
                self.last = Some((received, seconds));
            }
            Some(_) => {}
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a non-negative speed, rounded to whole bytes"
        )]
        self.rate.map(|rate| rate.round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use quickie_domain_library::{Download, DownloadId, DownloadState, Timestamp};
    use url::Url;

    use super::{Action, Line, Phase, RateMeter, line, on_enter};

    fn download(state: DownloadState) -> Download {
        Download {
            id: DownloadId::new(3),
            url: Url::parse("https://files.example.com/a.dmg").unwrap(),
            file_name: "a.dmg".to_owned(),
            state,
            started_at: Timestamp::from_millis(0),
        }
    }

    fn running(received: u64, total: Option<u64>) -> Download {
        download(DownloadState::InProgress { received, total })
    }

    #[test]
    fn a_download_that_has_nothing_yet_is_starting() {
        let line = line(&running(0, None), None);

        assert_eq!(
            line,
            Line {
                id: DownloadId::new(3),
                name: "a.dmg".to_owned(),
                detail: "Starting…".to_owned(),
                fraction: None,
                phase: Phase::Running,
            }
        );
    }

    #[test]
    fn a_running_download_says_what_is_known() {
        let full = line(&running(184_200_000, Some(240_000_000)), Some(4_200_000));
        let no_size = line(&running(1_500_000, None), Some(500_000));
        let no_rate = line(&running(1_500_000, Some(3_000_000)), None);

        assert_eq!(full.detail, "184.2 MB of 240 MB · 4.2 MB/s · 14 s left");
        assert_eq!(full.fraction, Some(0.7675));
        assert_eq!(no_size.detail, "1.5 MB · 500 KB/s");
        assert_eq!(no_size.fraction, None);
        assert_eq!(no_rate.detail, "1.5 MB of 3 MB");
        assert_eq!(no_rate.fraction, Some(0.5));
    }

    #[test]
    fn a_stalled_download_shows_no_rate_and_a_zero_size_no_bar() {
        assert_eq!(
            line(&running(0, Some(10)), Some(0)).detail,
            "0 bytes of 10 bytes"
        );
        assert_eq!(line(&running(0, Some(0)), None).fraction, None);
        assert_eq!(line(&running(20, Some(10)), None).fraction, Some(1.0));
    }

    #[test]
    fn ended_downloads_say_how_they_ended() {
        let done = line(&download(DownloadState::Finished), None);
        let failed = line(&download(DownloadState::Failed("disk full".into())), None);
        let cancelled = line(&download(DownloadState::Cancelled), None);

        assert_eq!(
            (done.phase, done.detail.as_str()),
            (Phase::Done, "Done · files.example.com")
        );
        assert_eq!(
            (failed.phase, failed.detail.as_str()),
            (Phase::Failed, "Failed · disk full")
        );
        assert_eq!(
            (cancelled.phase, cancelled.detail.as_str()),
            (Phase::Cancelled, "Cancelled · files.example.com")
        );
    }

    #[test]
    fn a_download_without_a_host_says_only_its_state() {
        let mut blank = download(DownloadState::Finished);
        blank.url = Url::parse("about:blank").unwrap();

        assert_eq!(line(&blank, None).detail, "Done");
    }

    #[test]
    fn each_phase_has_its_buttons() {
        assert_eq!(Phase::Running.actions(), [Action::Cancel]);
        assert_eq!(Phase::Done.actions(), [Action::Open, Action::Reveal]);
        assert_eq!(Phase::Failed.actions(), [Action::Retry]);
        assert_eq!(Phase::Cancelled.actions(), [Action::Retry]);
    }

    #[test]
    fn enter_opens_or_shows_a_finished_file_and_retries_an_ended_one() {
        assert_eq!(on_enter(Phase::Done, false), Some(Action::Open));
        assert_eq!(on_enter(Phase::Done, true), Some(Action::Reveal));
        assert_eq!(on_enter(Phase::Failed, false), Some(Action::Retry));
        assert_eq!(on_enter(Phase::Cancelled, false), Some(Action::Retry));
        assert_eq!(on_enter(Phase::Cancelled, true), None);
        assert_eq!(on_enter(Phase::Running, false), None);
        assert_eq!(on_enter(Phase::Running, true), None);
    }

    #[test]
    fn the_rate_needs_two_samples_far_enough_apart() {
        let mut meter = RateMeter::default();

        assert_eq!(meter.sample(0, 0.0), None);
        assert_eq!(meter.sample(1000, 0.1), None);
        assert_eq!(meter.sample(1000, 0.5), Some(2000));
    }

    #[test]
    fn the_rate_is_smoothed() {
        let mut meter = RateMeter::default();
        meter.sample(0, 0.0);
        meter.sample(1000, 1.0);

        let rate = meter.sample(3000, 2.0);

        assert_eq!(rate, Some(1300));
    }
}
