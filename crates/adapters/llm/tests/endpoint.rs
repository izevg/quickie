//! The client through the development engine's real network stack, against a one-shot
//! OpenAI-compatible server on 127.0.0.1 that streams canned answers. No real LLM is asked, and
//! no request leaves the machine.
// Test code throughout, helpers included, so they may unwrap like tests do (`.clippy.toml`).
#![cfg(test)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use quickie_application::{ChatEvent, ChatRequest, Ended, LlmError, LlmPort, StopSignal};
use quickie_domain_agents::{
    ApiKey, BaseUrl, LlmEndpoint, ModelId, PageText, Prompt, Question, Request, TokenBudget,
};
use quickie_engine_webkit::UrlSessionHttp;
use quickie_llm::OpenAiClient;
use url::Url;

/// What the server read: the request line, the headers in lowercase, the body.
struct Received {
    line: String,
    headers: Vec<String>,
    body: String,
}

/// Serves one request: answers `status` with `content_type`, writes each of `pieces` after a
/// pause, then holds the connection for `hold`. Returns the endpoint's base URL and what the
/// server read.
fn serve(
    status: &'static str,
    content_type: &'static str,
    pieces: Vec<Vec<u8>>,
    hold: Duration,
) -> (BaseUrl, Receiver<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (report, received) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut headers = Vec::new();
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            if header.trim().is_empty() {
                break;
            }
            headers.push(header.trim().to_lowercase());
        }
        let length = headers
            .iter()
            .find_map(|header| header.strip_prefix("content-length: "))
            .map_or(0, |length| length.parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let _ = report.send(Received {
            line: line.trim().to_owned(),
            headers,
            body: String::from_utf8(body).unwrap(),
        });
        let mut stream = reader.into_inner();
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        for piece in pieces {
            thread::sleep(Duration::from_millis(20));
            if stream
                .write_all(&piece)
                .and_then(|()| stream.flush())
                .is_err()
            {
                return;
            }
        }
        thread::sleep(hold);
    });
    let base = BaseUrl::try_from(format!("http://127.0.0.1:{port}/v1").as_str()).unwrap();
    (base, received)
}

fn sse(content: &str) -> String {
    format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{content}\"}}}}]}}\n\n")
}

fn prompt() -> Prompt {
    let request = Request::Question(Question::try_from("What does it say?").unwrap());
    let page = PageText::new(
        Url::parse("https://example.com/").unwrap(),
        "T",
        "Hello world",
    );
    Prompt::assemble(&request, &[page], TokenBudget::DEFAULT).unwrap()
}

/// Asks the endpoint at `base`, with `key`, and collects every event.
fn ask(
    base: BaseUrl,
    key: Option<&ApiKey>,
    stop: &StopSignal,
    on_event: &mut dyn FnMut(ChatEvent),
) -> Result<Ended, LlmError> {
    let endpoint = LlmEndpoint::new(base, ModelId::try_from("local-model").unwrap());
    let prompt = prompt();
    OpenAiClient::new(UrlSessionHttp::new()).chat(
        &ChatRequest {
            endpoint: &endpoint,
            key,
            prompt: &prompt,
        },
        stop,
        on_event,
    )
}

#[test]
fn an_answer_streams_token_by_token_through_the_network_stack() {
    let stream = format!(
        "{}{}data: {{\"choices\":[],\"usage\":{{\"completion_tokens\":2}}}}\n\ndata: [DONE]\n\n",
        sse("Привет"),
        sse(", мир")
    );
    let bytes = stream.into_bytes();
    // Cut inside the two bytes of "и", then between the first two events, then inside "м".
    let pieces = vec![
        bytes[..44].to_vec(),
        bytes[44..58].to_vec(),
        bytes[58..100].to_vec(),
        bytes[100..].to_vec(),
    ];
    let (base, _) = serve("200 OK", "text/event-stream", pieces, Duration::ZERO);
    let mut events = Vec::new();

    let ended = ask(base, None, &StopSignal::new(), &mut |event| {
        events.push(event);
    });

    assert_eq!(ended, Ok(Ended::Complete));
    assert_eq!(
        events,
        [
            ChatEvent::Delta("Привет".to_owned()),
            ChatEvent::Delta(", мир".to_owned()),
            ChatEvent::Usage {
                completion_tokens: 2
            },
        ]
    );
}

#[test]
fn the_request_is_a_streaming_chat_completion_with_the_key() {
    let (base, received) = serve(
        "200 OK",
        "text/event-stream",
        vec![b"data: [DONE]\n\n".to_vec()],
        Duration::ZERO,
    );
    let key = ApiKey::try_from("sk-local-test").unwrap();

    let _ = ask(base, Some(&key), &StopSignal::new(), &mut |_| {});

    let request = received.recv().unwrap();
    assert_eq!(request.line, "POST /v1/chat/completions HTTP/1.1");
    assert!(
        request
            .headers
            .contains(&"authorization: bearer sk-local-test".to_owned())
    );
    assert!(
        request
            .headers
            .contains(&"content-type: application/json".to_owned())
    );
    assert!(request.body.contains("\"stream\":true"));
    assert!(request.body.contains("\"model\":\"local-model\""));
}

#[test]
fn a_request_without_a_key_has_no_authorization() {
    let (base, received) = serve(
        "200 OK",
        "text/event-stream",
        vec![b"data: [DONE]\n\n".to_vec()],
        Duration::ZERO,
    );

    let _ = ask(base, None, &StopSignal::new(), &mut |_| {});

    let request = received.recv().unwrap();
    assert!(
        !request
            .headers
            .iter()
            .any(|header| header.starts_with("authorization"))
    );
}

#[test]
fn a_refused_key_reports_the_endpoints_message() {
    let body =
        br#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#;
    let (base, _) = serve(
        "401 Unauthorized",
        "application/json",
        vec![body.to_vec()],
        Duration::ZERO,
    );

    let ended = ask(base, None, &StopSignal::new(), &mut |_| {});

    assert_eq!(
        ended,
        Err(LlmError::Status {
            status: 401,
            message: "Incorrect API key provided".to_owned(),
        })
    );
}

#[test]
fn stop_ends_an_answer_the_server_is_still_writing() {
    let (base, _) = serve(
        "200 OK",
        "text/event-stream",
        vec![sse("First").into_bytes()],
        Duration::from_secs(10),
    );
    let stop = StopSignal::new();
    let stopper = stop.clone();
    let mut events = Vec::new();
    let started = Instant::now();

    let ended = ask(base, None, &stop, &mut |event| {
        events.push(event);
        stopper.stop();
    });

    assert_eq!(ended, Ok(Ended::Stopped));
    assert_eq!(events, [ChatEvent::Delta("First".to_owned())]);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn models_are_listed_through_the_network_stack() {
    let body = br#"{"object":"list","data":[{"id":"llama3.1:8b"},{"id":"phi3"}]}"#;
    let (base, received) = serve(
        "200 OK",
        "application/json",
        vec![body.to_vec()],
        Duration::ZERO,
    );

    let models = OpenAiClient::new(UrlSessionHttp::new()).models(&base, None);

    assert_eq!(
        models,
        Ok(vec![
            ModelId::try_from("llama3.1:8b").unwrap(),
            ModelId::try_from("phi3").unwrap()
        ])
    );
    assert_eq!(received.recv().unwrap().line, "GET /v1/models HTTP/1.1");
}
