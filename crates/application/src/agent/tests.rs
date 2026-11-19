//! The agent use cases against in-memory fakes; no request leaves the process.

mod answer;
mod fakes;
mod local;
mod settings;

use quickie_domain_agents::{ApiKey, BaseUrl, LlmEndpoint, ModelId, PageText, Question, Request};
use url::Url;

fn base(text: &str) -> BaseUrl {
    BaseUrl::try_from(text).unwrap()
}

fn model(text: &str) -> ModelId {
    ModelId::try_from(text).unwrap()
}

fn key(text: &str) -> ApiKey {
    ApiKey::try_from(text).unwrap()
}

/// A hosted endpoint, without a key.
fn hosted() -> LlmEndpoint {
    LlmEndpoint::new(base("https://api.example.com/v1"), model("small"))
}

fn page(text: &str) -> PageText {
    PageText::new(Url::parse("https://example.com/").unwrap(), "Example", text)
}

fn question(text: &str) -> Request {
    Request::Question(Question::try_from(text).unwrap())
}
