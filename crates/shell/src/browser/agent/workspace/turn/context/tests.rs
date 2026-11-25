use std::cell::RefCell;
use std::rc::Rc;

use quickie_application::{
    AgentPage, AgentTool, AgentToolCall, Ended, Flow, HttpError, HttpEvent, HttpMethod,
    HttpRequest, LlmPort, StopSignal, ToolChatReply, ToolChatRequest,
};
use quickie_domain_agents::{BaseUrl, ModelId};
use url::Url;

use super::*;

const PAGE_RESULT: &str = "private-page-result";

#[derive(Clone, Default)]
struct RecordedHttp(Rc<RefCell<Vec<HttpRequest>>>);

impl HttpPort for RecordedHttp {
    fn get(&self, _: &Url) -> Result<Vec<u8>, HttpError> {
        panic!("the agent sends completions as POST requests");
    }

    fn stream(
        &self,
        request: &HttpRequest,
        _: &StopSignal,
        on_event: &mut dyn FnMut(HttpEvent<'_>) -> Flow,
    ) -> Result<Ended, HttpError> {
        self.0.borrow_mut().push(request.clone());
        on_event(HttpEvent::Status(200));
        on_event(HttpEvent::Body(br#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Done."}}]}"#));
        Ok(Ended::Complete)
    }
}

fn endpoint(base: &str, model: &str) -> LlmEndpoint {
    LlmEndpoint::new(
        BaseUrl::try_from(base).unwrap(),
        ModelId::try_from(model).unwrap(),
    )
}

fn answered(http: RecordedHttp, endpoint: &LlmEndpoint) -> ModelContext<RecordedHttp> {
    let mut context = ModelContext::new(http, Some(endpoint));
    assert!(context.conversation.begin("What is on the page?"));
    assert!(
        context
            .conversation
            .accept(&ToolChatReply::Call(AgentToolCall {
                id: "read_1".to_owned(),
                tool: AgentTool::ReadPage(AgentPage::Source),
            }))
    );
    assert!(
        context
            .conversation
            .tool_result("read_1", PAGE_RESULT.to_owned())
    );
    assert!(
        context
            .conversation
            .accept(&ToolChatReply::Answer("A private answer.".to_owned()))
    );
    context
}

fn send(
    context: &ModelContext<RecordedHttp>,
    endpoint: &LlmEndpoint,
    client: &OpenAiClient<RecordedHttp>,
    stop: &StopSignal,
) {
    let _ = client.complete_with_tools(
        &ToolChatRequest {
            endpoint,
            key: None,
            messages: context.conversation.messages(),
            automation: false,
        },
        stop,
    );
}

fn first_body(http: &RecordedHttp, endpoint: &LlmEndpoint) -> String {
    let all = http.0.borrow();
    let [sent] = all.as_slice() else {
        panic!("expected a single model request");
    };
    assert_eq!(sent.url.origin(), endpoint.base().as_url().origin());
    let HttpMethod::Post(body) = &sent.method else {
        panic!("expected an agent completion");
    };
    String::from_utf8(body.clone()).unwrap()
}

#[test]
fn a_different_origin_receives_only_the_new_question() {
    let http = RecordedHttp::default();
    let original = endpoint("https://first.example/v1", "one");
    let next = endpoint("https://second.example/v1", "one");
    let mut context = answered(http.clone(), &original);

    assert!(context.bind(Some(&next), &http));
    assert!(context.conversation.begin("A fresh question."));
    send(&context, &next, &context.llm.for_turn(), &StopSignal::new());

    let body = first_body(&http, &next);
    assert!(body.contains("A fresh question."));
    assert!(!body.contains(PAGE_RESULT));
    assert!(!body.contains("A private answer."));
}

#[test]
fn changing_model_and_path_at_the_same_origin_preserves_page_history() {
    let http = RecordedHttp::default();
    let original = endpoint("https://example.test/v1", "one");
    let next = endpoint("https://example.test:443/other", "two");
    let mut context = answered(http.clone(), &original);

    assert!(!context.bind(Some(&next), &http));
    assert!(context.conversation.begin("A follow-up."));
    send(&context, &next, &context.llm.for_turn(), &StopSignal::new());

    let body = first_body(&http, &next);
    assert!(body.contains("A follow-up."));
    assert!(body.contains(PAGE_RESULT));
}

#[test]
fn removing_then_restoring_the_same_origin_does_not_restore_page_history() {
    let http = RecordedHttp::default();
    let original = endpoint("https://example.com/v1", "one");
    let mut context = answered(http.clone(), &original);

    assert!(context.bind(None, &http));
    assert!(!context.bind(None, &http));
    assert!(context.bind(Some(&original), &http));
    assert!(context.conversation.begin("A fresh question."));
    send(
        &context,
        &original,
        &context.llm.for_turn(),
        &StopSignal::new(),
    );

    assert!(!first_body(&http, &original).contains(PAGE_RESULT));
}

#[test]
fn scheme_and_port_are_part_of_the_origin() {
    let http = RecordedHttp::default();
    let plain = endpoint("http://localhost:11434/v1", "one");
    let tls = endpoint("https://localhost:11434/v1", "one");
    let other_port = endpoint("https://localhost:11435/v1", "one");
    let mut context = answered(http.clone(), &plain);

    assert!(context.bind(Some(&tls), &http));
    assert!(context.bind(Some(&other_port), &http));
}
