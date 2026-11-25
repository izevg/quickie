//! What the agent's UI says, in plain words: the speed readout, the badge, the model chip, and
//! why something failed. An endpoint's own message is untrusted text of any length: it is cut
//! short and always shown as plain text, never as markup.

use quickie_application::{AskError, LlmError};
use quickie_domain_agents::{LlmEndpoint, Speed};

/// The most of an endpoint's own message shown, in characters.
const MAX_MESSAGE_CHARS: usize = 200;

/// The readout under a finished answer: `24 tok/s · 0.4 s TTFT`, measured; what was not
/// measured is left out, and nothing measured is empty.
#[must_use]
pub fn speed(speed: Speed) -> String {
    let rate = speed
        .tokens_per_s
        .map(|rate| format!("{} tok/s", number(rate)));
    let first = speed
        .ttft_ms
        .map(|millis| format!("{} s TTFT", seconds(millis)));
    [rate, first]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// What the agent is doing, for its badge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Activity {
    /// Reading the page, before the request goes out.
    Reading,
    /// Waiting for the first token.
    Asking,
    /// Answering, this fast so far when measured.
    Answering(Option<f64>),
}

/// The badge's text: `Agent · reading page`, `Agent · answering · 24 tok/s`.
#[must_use]
pub fn badge(activity: Activity) -> String {
    match activity {
        Activity::Reading => "Agent · reading page".to_owned(),
        Activity::Asking => "Agent · asking".to_owned(),
        Activity::Answering(None) => "Agent · answering".to_owned(),
        Activity::Answering(Some(rate)) => format!("Agent · answering · {} tok/s", number(rate)),
    }
}

/// The model chip: the model, and where it runs (`local`, or the endpoint's host).
#[must_use]
pub fn model_chip(endpoint: &LlmEndpoint) -> String {
    let place = if endpoint.base().is_loopback() {
        "local".to_owned()
    } else {
        endpoint
            .base()
            .as_url()
            .host_str()
            .unwrap_or_default()
            .to_owned()
    };
    format!("{} · {place}", endpoint.model().as_str())
}

/// Why asking failed.
#[must_use]
pub fn ask_error(error: &AskError) -> String {
    match error {
        AskError::KeyMissing(_) => {
            "The API key is gone from the Keychain. Set it again in Agent Settings.".to_owned()
        }
        AskError::Secret(error) => format!("The Keychain could not be read: {}.", error.0),
        AskError::Llm(error) => llm_error(error),
    }
}

/// Why the endpoint failed, as the user can act on it.
#[must_use]
pub fn llm_error(error: &LlmError) -> String {
    match error {
        LlmError::Http(error) => format!(
            "The endpoint can't be reached ({}). Check the address, and that the server is \
             running.",
            cut(&error.0)
        ),
        LlmError::Status { status, message } => status_error(*status, message),
        LlmError::ToolsUnavailable => {
            "The endpoint does not support native agent tools.".to_owned()
        }
        LlmError::Api(message) => format!("The endpoint failed mid-answer: {}", cut(message)),
        LlmError::Malformed(_) => {
            "The endpoint returned an invalid response for the selected model.".to_owned()
        }
        LlmError::TooLarge(what) => format!("The endpoint sent too much: {}.", cut(what)),
    }
}

fn status_error(status: u16, message: &str) -> String {
    let said = if message.trim().is_empty() {
        String::new()
    } else {
        format!(" It said: {}", cut(message))
    };
    match status {
        300..=399 => format!(
            "The endpoint redirected elsewhere ({status}). Quickie doesn't follow redirects \
             with your page; set the address it redirects to."
        ),
        401 | 403 => format!("The endpoint refused the API key ({status}). Check the key.{said}"),
        404 => format!("The endpoint could not find the API route or selected model (404).{said}"),
        429 => format!("The endpoint is rate limiting (429). Try again in a moment.{said}"),
        _ => format!("The endpoint answered {status}.{said}"),
    }
}

/// `text` on one line and at most [`MAX_MESSAGE_CHARS`] characters.
fn cut(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= MAX_MESSAGE_CHARS {
        return line;
    }
    let kept: String = line.chars().take(MAX_MESSAGE_CHARS - 1).collect();
    format!("{kept}…")
}

/// A rate: whole numbers from 10 up, one decimal below.
fn number(value: f64) -> String {
    if value >= 10.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// Milliseconds as seconds: one decimal below 10 s, whole from there.
fn seconds(millis: u64) -> String {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a wait in milliseconds stays far below 2^52"
    )]
    let seconds = millis as f64 / 1000.0;
    number(seconds)
}

#[cfg(test)]
mod tests {
    use quickie_application::{AskError, HttpError, LlmError, SecretError};
    use quickie_domain_agents::{BaseUrl, KeyRef, LlmEndpoint, ModelId, Speed};

    use super::{Activity, ask_error, badge, llm_error, model_chip, speed};

    fn status(status: u16, message: &str) -> String {
        llm_error(&LlmError::Status {
            status,
            message: message.to_owned(),
        })
    }

    #[test]
    fn the_readout_shows_what_was_measured() {
        let both = Speed {
            ttft_ms: Some(400),
            tokens_per_s: Some(24.4),
        };
        let slow = Speed {
            ttft_ms: Some(12_300),
            tokens_per_s: Some(3.25),
        };

        assert_eq!(speed(both), "24 tok/s · 0.4 s TTFT");
        assert_eq!(speed(slow), "3.2 tok/s · 12 s TTFT");
        assert_eq!(
            speed(Speed {
                ttft_ms: Some(50),
                tokens_per_s: None
            }),
            "0.1 s TTFT"
        );
        assert_eq!(
            speed(Speed {
                ttft_ms: None,
                tokens_per_s: None
            }),
            ""
        );
    }

    #[test]
    fn the_badge_says_what_the_agent_does() {
        assert_eq!(badge(Activity::Reading), "Agent · reading page");
        assert_eq!(badge(Activity::Asking), "Agent · asking");
        assert_eq!(badge(Activity::Answering(None)), "Agent · answering");
        assert_eq!(
            badge(Activity::Answering(Some(24.0))),
            "Agent · answering · 24 tok/s"
        );
    }

    #[test]
    fn the_chip_names_the_model_and_where_it_runs() {
        let local = LlmEndpoint::new(
            BaseUrl::try_from("http://127.0.0.1:11434/v1").unwrap(),
            ModelId::try_from("qwen3:32b").unwrap(),
        );
        let hosted = LlmEndpoint::new(
            BaseUrl::try_from("https://api.example.com/v1").unwrap(),
            ModelId::try_from("gpt-x").unwrap(),
        );

        assert_eq!(model_chip(&local), "qwen3:32b · local");
        assert_eq!(model_chip(&hosted), "gpt-x · api.example.com");
    }

    #[test]
    fn a_refused_key_says_so() {
        assert_eq!(
            status(401, "Incorrect API key provided"),
            "The endpoint refused the API key (401). Check the key. It said: Incorrect API key \
             provided"
        );
        assert!(status(403, "").starts_with("The endpoint refused the API key (403)"));
    }

    #[test]
    fn a_redirect_is_refused_in_plain_words() {
        for code in [301, 302, 307, 308] {
            assert!(
                status(code, "x").starts_with(&format!(
                    "The endpoint redirected elsewhere ({code}). Quickie doesn't follow"
                )),
                "{code}"
            );
        }
    }

    #[test]
    fn other_statuses_name_the_status() {
        assert_eq!(
            status(404, "Model not found"),
            "The endpoint could not find the API route or selected model (404). It said: Model not found"
        );
        assert!(status(429, "").starts_with("The endpoint is rate limiting (429)"));
        assert_eq!(status(500, ""), "The endpoint answered 500.");
        assert_eq!(
            status(299, "odd"),
            "The endpoint answered 299. It said: odd"
        );
        assert_eq!(
            status(400, "bad"),
            "The endpoint answered 400. It said: bad"
        );
    }

    #[test]
    fn an_endpoint_message_is_one_short_line() {
        let long = format!("line one\nline two {}", "x".repeat(500));

        let shown = status(500, &long);

        assert!(!shown.contains('\n'));
        assert!(shown.contains("line one line two"));
        assert!(shown.ends_with('…'));
        assert_eq!(
            shown.chars().count(),
            "The endpoint answered 500. It said: ".chars().count() + 200
        );
        assert_eq!(status(500, &"y".repeat(200)).matches('y').count(), 200);
    }

    #[test]
    fn unreachable_and_malformed_say_what_to_check() {
        let unreachable = llm_error(&LlmError::Http(HttpError(
            "Could not connect to the server.".to_owned(),
        )));

        assert_eq!(
            unreachable,
            "The endpoint can't be reached (Could not connect to the server.). Check the \
             address, and that the server is running."
        );
        assert_eq!(
            llm_error(&LlmError::Malformed("x".to_owned())),
            "The endpoint returned an invalid response for the selected model."
        );
        assert_eq!(
            llm_error(&LlmError::Api("overloaded".to_owned())),
            "The endpoint failed mid-answer: overloaded"
        );
        assert_eq!(
            llm_error(&LlmError::TooLarge("an answer".to_owned())),
            "The endpoint sent too much: an answer."
        );
    }

    #[test]
    fn keychain_trouble_points_at_settings() {
        let base = BaseUrl::try_from("https://api.example.com/v1").unwrap();

        assert!(
            ask_error(&AskError::KeyMissing(KeyRef::for_origin(&base)))
                .contains("Set it again in Agent Settings")
        );
        assert_eq!(
            ask_error(&AskError::Secret(SecretError("locked".to_owned()))),
            "The Keychain could not be read: locked."
        );
        assert_eq!(
            ask_error(&AskError::Llm(LlmError::Api("x".to_owned()))),
            "The endpoint failed mid-answer: x"
        );
    }
}
