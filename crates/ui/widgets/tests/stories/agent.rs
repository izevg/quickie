//! The agent group: the answer panel while it streams, cites, and fails, and the badge.

use std::cell::RefCell;
use std::rc::Rc;

use quickie_application::{AnswerEvent, AskError, LlmError};
use quickie_domain_agents::{Citation, Speed};
use quickie_ui_shared::Trigger;
use quickie_ui_shared::stories::{LIGHT, LIGHT_DARK, Scene, Story, Theme};
use quickie_ui_widgets::{
    Activity, AgentAnswerPanel, AgentBadge, AgentFrame, AnswerHeader, PanelRequest,
};

const fn panel(state: &'static str, themes: &'static [Theme], scene: fn(&mut Scene)) -> Story {
    Story {
        group: "agent",
        widget: "answer-panel",
        state,
        themes,
        window: (1280.0, 800.0),
        scene,
    }
}

pub const STORIES: &[Story] = &[
    panel("unconfigured", LIGHT_DARK, unconfigured),
    panel("short", LIGHT, short),
    panel("reading", LIGHT, reading),
    panel("streaming", LIGHT_DARK, streaming),
    panel("citations", LIGHT_DARK, citations),
    panel("error", LIGHT_DARK, error),
    panel("russian", LIGHT, russian),
    Story {
        group: "agent",
        widget: "agent-badge",
        state: "answering",
        themes: LIGHT_DARK,
        window: (1024.0, 640.0),
        scene: badge,
    },
    Story {
        group: "agent",
        widget: "agent-badge",
        state: "reading",
        themes: LIGHT,
        window: (1024.0, 640.0),
        scene: badge_reading,
    },
    Story {
        group: "agent",
        widget: "agent-frame",
        state: "acting",
        themes: LIGHT_DARK,
        window: (640.0, 400.0),
        scene: frame,
    },
];

fn open(scene: &mut Scene) -> Rc<AgentAnswerPanel> {
    let panel = Rc::new(AgentAnswerPanel::new(scene.mtm()));
    panel.show(scene.host(), Trigger::Keyboard);
    scene.keep(Rc::clone(&panel));
    panel
}

fn header(title: &str) -> AnswerHeader {
    AnswerHeader {
        title: title.to_owned(),
        model: "qwen3-8b · local".to_owned(),
    }
}

fn tokens(text: &str) -> Vec<AnswerEvent> {
    text.split_inclusive(' ')
        .map(|word| AnswerEvent::Token(word.to_owned()))
        .collect()
}

fn cite(quote: &str) -> AnswerEvent {
    AnswerEvent::Citation(Citation {
        quote: quote.to_owned(),
        anchor: quote.to_lowercase(),
    })
}

const ANSWER: &str = "**Short answer:** the page argues that sovereignty is *limited by law*.\n\n\
    - Courts bind the sovereign\n- No power stands above the law\n\nIt says so twice: ";

/// Clicks the button `VoiceOver` reads as `label` and checks what the panel asked.
fn press(
    scene: &mut Scene,
    asked: &Rc<RefCell<Vec<PanelRequest>>>,
    label: &str,
    expected: PanelRequest,
) {
    let Some(button) = scene.view_labelled(label) else {
        scene.check(false, format!("a button reads {label:?}"));
        return;
    };
    scene.click(&button);
    let last = asked.borrow().last().cloned();
    scene.check_eq(last, Some(expected), format!("a click on {label:?}"));
}

fn listen(panel: &AgentAnswerPanel) -> Rc<RefCell<Vec<PanelRequest>>> {
    let asked = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&asked);
    panel.on_request(move |request| log.borrow_mut().push(request));
    asked
}

/// No endpoint: a centred tile, the settings in the agent color, looking for a local server.
fn unconfigured(scene: &mut Scene) {
    let panel = open(scene);
    let asked = listen(&panel);
    panel.unconfigured();
    scene.check(!panel.is_working(), "nothing is asked");
    scene.check_eq(panel.return_request(), None, "⏎ does nothing here");
    press(scene, &asked, "Agent Settings…", PanelRequest::Settings);
    press(scene, &asked, "Detect Local LLM", PanelRequest::DetectLocal);
}

/// A short answer keeps the panel short; the footer offers a follow-up and the copy.
fn short(scene: &mut Scene) {
    let panel = open(scene);
    let asked = listen(&panel);
    panel.start(&header("Is this page about law?"));
    let mut events = tokens("Yes: it argues that the sovereign is *bound* by law.");
    events.push(AnswerEvent::Done(Speed {
        ttft_ms: Some(380),
        tokens_per_s: Some(42.0),
    }));
    panel.receive(&events);
    let height = panel.view().frame().size.height;
    scene.check(
        height < 360.0,
        format!("a short answer keeps the panel short ({height} pt)"),
    );
    scene.check_eq(
        panel.return_request(),
        Some(PanelRequest::FollowUp),
        "⏎ follows up",
    );
    press(scene, &asked, "Follow-up", PanelRequest::FollowUp);
    press(scene, &asked, "Copy Answer", PanelRequest::Copy);
}

fn reading(scene: &mut Scene) {
    let panel = open(scene);
    panel.start(&header("What does this page argue?"));
    scene.check(panel.is_working(), "the agent works");
}

fn streaming(scene: &mut Scene) {
    let panel = open(scene);
    panel.start(&header("What does this page argue?"));
    panel.set_activity(Activity::Answering(Some(24.0)));
    panel.receive(&tokens(ANSWER));
    scene.check(panel.is_working(), "still answering");
    scene.check(
        panel.answer().contains("Short answer:"),
        "the Markdown arrives as text",
    );
}

/// Citations become chips; clicking one asks the browser to find its passage.
fn citations(scene: &mut Scene) {
    let panel = open(scene);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&asked);
    panel.on_request(move |request| log.borrow_mut().push(request));
    panel.start(&header("What does this page argue?"));
    let mut events = tokens(ANSWER);
    events.push(cite("Sovereign power is limited by law"));
    events.push(AnswerEvent::Token(" and ".to_owned()));
    events.push(cite("no sovereign stands above them"));
    events.push(AnswerEvent::Token(".".to_owned()));
    events.push(AnswerEvent::Done(Speed {
        ttft_ms: Some(420),
        tokens_per_s: Some(31.5),
    }));
    panel.receive(&events);
    scene.check(!panel.is_working(), "the answer is done");
    if let Some(chip) = scene.view_labelled("“Sovereign power is limited by law”") {
        scene.click(&chip);
        scene.check_eq(
            asked.borrow().first().cloned(),
            Some(PanelRequest::Cite(
                "sovereign power is limited by law".to_owned(),
            )),
            "a click on a chip finds its passage",
        );
    } else {
        scene.check(false, "the first citation has a chip");
    }
}

/// A failed answer says why in red, under a warning, and offers to try again.
fn error(scene: &mut Scene) {
    let panel = open(scene);
    let asked = listen(&panel);
    panel.start(&header("Summarize this page"));
    let mut events = tokens("The page is about ");
    events.push(AnswerEvent::Error(AskError::Llm(LlmError::Status {
        status: 401,
        message: "Incorrect API key provided".to_owned(),
    })));
    panel.receive(&events);
    scene.check(!panel.is_working(), "a failed answer stops");
    scene.check(
        panel.answer().contains("API key"),
        "the reason shows under what came",
    );
    scene.check_eq(
        panel.return_request(),
        Some(PanelRequest::Retry),
        "⏎ retries",
    );
    press(scene, &asked, "Retry", PanelRequest::Retry);
}

fn russian(scene: &mut Scene) {
    let panel = open(scene);
    panel.start(&header("О чём эта страница?"));
    let mut events =
        tokens("Страница объясняет **суверенитет**: власть государства ограничена правом, и ");
    events.push(cite("суверен подчиняется праву"));
    events.push(AnswerEvent::Done(Speed {
        ttft_ms: Some(900),
        tokens_per_s: None,
    }));
    panel.receive(&events);
}

fn badge(scene: &mut Scene) {
    let badge = AgentBadge::new(scene.mtm());
    badge.show(scene.host(), "Agent · answering · 24 tok/s", 220.0);
    scene.check(badge.is_shown(), "the badge shows");
    scene.keep(badge);
}

fn badge_reading(scene: &mut Scene) {
    let badge = AgentBadge::new(scene.mtm());
    badge.show(scene.host(), "Agent · reading page", 1024.0);
    scene.keep(badge);
}

/// While the agent acts the window is framed in its color, and the frame takes no click: the
/// page under it still gets them.
fn frame(scene: &mut Scene) {
    let frame = AgentFrame::new(scene.mtm());
    frame.show(scene.host());
    let badge = AgentBadge::new(scene.mtm());
    badge.show(scene.host(), "Agent · reading page", 640.0);
    scene.check(frame.is_shown(), "the frame shows");
    let size = scene.host().bounds().size;
    let hit = scene
        .host()
        .hitTest(objc2_foundation::NSPoint::new(1.0, size.height / 2.0));
    let through = hit.is_none_or(|view| &*view != frame.view());
    scene.check(through, "a click on the frame goes through to the page");
    frame.show(scene.host());
    scene.check_eq(
        scene.host().subviews().len(),
        2,
        "showing it again adds nothing",
    );
    scene.crop_to(frame.view());
    scene.keep(frame);
    scene.keep(badge);
}
