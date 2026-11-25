//! The agent group: the answer panel while it streams, cites, and fails, and the badge.

use std::cell::RefCell;
use std::rc::Rc;

use quickie_application::{AnswerEvent, AskError, LlmError};
use quickie_domain_agents::{Citation, Speed};
use quickie_ui_shared::Trigger;
use quickie_ui_shared::stories::{Key, LIGHT, LIGHT_DARK, Scene, Story, Theme};
use quickie_ui_widgets::{
    Activity, AgentAnswerPanel, AgentBadge, AgentFrame, AgentWorkspacePanel, AnswerHeader,
    PanelRequest, WorkspaceCitation, WorkspacePanelRequest, WorkspaceToolStatus,
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
    workspace_story("idle", (840.0, 440.0), workspace_idle),
    workspace_story("unconfigured", (840.0, 440.0), workspace_unconfigured),
    workspace_story("running", (1080.0, 700.0), workspace_running),
    workspace_story("success", (1080.0, 700.0), workspace_success),
    workspace_story("error", (840.0, 440.0), workspace_error),
    workspace_story("stopped", (1080.0, 700.0), workspace_stopped),
    workspace_story("history", (840.0, 440.0), workspace_history),
    workspace_story("citations", (1080.0, 700.0), workspace_citations),
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

const fn workspace_story(state: &'static str, window: (f64, f64), scene: fn(&mut Scene)) -> Story {
    Story {
        group: "agent",
        widget: "agent-workspace",
        state,
        themes: &Theme::ALL,
        window,
        scene,
    }
}

fn workspace(scene: &mut Scene) -> Rc<AgentWorkspacePanel> {
    use objc2_app_kit::NSView;

    let panel = AgentWorkspacePanel::new(scene.mtm());
    let preview = NSView::new(scene.mtm());
    preview.setHidden(true);
    panel.attach_page(&preview);
    panel.show_in(scene.host());
    panel.set_source("example.test");
    scene.keep(Rc::clone(&panel));
    panel
}

fn workspace_idle(scene: &mut Scene) {
    use objc2_app_kit::{NSButton, NSView};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    // Arrange
    let source = NSButton::new(scene.mtm());
    source.setTitle(&NSString::from_str("Source page action"));
    source.setFrame(NSRect::new(
        NSPoint::new(24.0, 24.0),
        NSSize::new(132.0, 32.0),
    ));
    scene.host().addSubview(&source);
    let panel = workspace(scene);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&asked);
    panel.on_request(move |request| log.borrow_mut().push(request));

    let card = panel.card_view().bounds().size;
    scene.check(
        (card.width - 336.0).abs() < 1.0 && (card.height - 240.0).abs() < 1.0,
        "the narrow answer card starts compact",
    );
    let hit_view = |view: &NSView| {
        let size = view.bounds().size;
        let point =
            view.convertPoint_toView(NSPoint::new(size.width / 2.0, size.height / 2.0), None);
        scene.host().hitTest(point).as_deref() == Some(view)
    };
    let source_hit = hit_view(&source);
    let close_hit = scene
        .view_labelled("Close Agent")
        .is_some_and(|close| hit_view(&close));
    scene.check(
        source_hit,
        "the overlay leaves source page controls clickable",
    );
    scene.check(
        close_hit,
        "the overlay receives clicks inside the Agent card",
    );
    scene.check(
        scene.window().contentView().as_deref() == Some(scene.host()),
        "the source window keeps its original content view",
    );
    scene.check(
        scene.host().hitTest(NSPoint::new(240.0, 100.0)).as_deref() == Some(scene.host()),
        "an empty source-page point also passes through the overlay",
    );
    source.removeFromSuperview();

    // Act
    if scene.view_labelled("Message the agent").is_some() {
        panel.focus_composer();
        scene.type_text("Read this page");
        scene.press(Key::Return);
    } else {
        scene.check(false, "composer is accessible");
    }

    // Assert
    scene.check_eq(
        asked.borrow().clone(),
        vec![WorkspacePanelRequest::Submit("Read this page".to_owned())],
        "Return submits the composer once",
    );
    scene.check(
        scene.view_labelled("Stop agent").is_none(),
        "Stop stays hidden while idle",
    );
}

fn workspace_unconfigured(scene: &mut Scene) {
    // Arrange
    let panel = workspace(scene);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&asked);
    panel.on_request(move |request| log.borrow_mut().push(request));

    // Act
    panel.set_unconfigured(true);
    if let Some(button) = scene.view_labelled("Agent Settings…") {
        scene.click(&button);
    }

    // Assert
    scene.check_eq(
        asked.borrow().clone(),
        vec![WorkspacePanelRequest::Settings],
        "setup opens settings",
    );
}

fn workspace_running(scene: &mut Scene) {
    // Arrange
    let panel = workspace(scene);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&asked);
    panel.on_request(move |request| log.borrow_mut().push(request));
    panel.append_prompt("Open the guide linked on this page");
    panel.set_activity("Opening the guide…", true);
    panel.start_tool(
        "Open link",
        "example.test/library/guides/really-long-name-that-truncates-cleanly-in-a-compact-tool-row",
    );

    workspace_follow_up(scene, &panel, &asked);

    // Act
    if let Some(button) = scene.view_labelled("Stop agent") {
        scene.click(&button);
    }

    // Assert
    scene.check_eq(
        asked.borrow().clone(),
        vec![WorkspacePanelRequest::Stop],
        "Stop cancels the active turn",
    );
}

fn workspace_success(scene: &mut Scene) {
    // Arrange
    let panel = workspace(scene);
    panel.append_prompt("Open the guide linked on this page");
    panel.set_activity("Thinking…", true);
    let row = panel.start_tool("Open link", "example.test/guide");

    // Act
    panel.finish_tool(row, WorkspaceToolStatus::Completed, "Navigation finished");
    panel.append_answer("The **guide** is open. You can [read more](https://example.test/guide).");
    panel.set_activity("", false);

    // Assert
    scene.check(
        scene
            .view_labelled("Open link, Completed, show details")
            .is_some(),
        "completion updates the invocation row",
    );
    scene.check(
        scene
            .view_labelled("Open link, Running, show details")
            .is_none(),
        "no stale running invocation remains",
    );
    scene.check(
        scene.view_labelled("Thinking…").is_none() && scene.view_labelled("Stop agent").is_none(),
        "the active controls clear when the answer arrives",
    );
}

fn workspace_error(scene: &mut Scene) {
    use objc2_app_kit::NSScrollView;

    // Arrange
    let panel = workspace(scene);
    panel.append_prompt("Open the guide");
    let row = panel.start_tool("Open link", "example.test/guide");
    panel.finish_tool(
        row,
        WorkspaceToolStatus::Failed,
        "Quickie found this link on an earlier revision of the source tab.",
    );
    panel.append_notice(
        "The page changed before the link opened. Check the source page and try again.",
        true,
    );
    panel.set_activity("", false);
    let compact_height = panel.card_view().bounds().size.height;

    // Act
    if let Some(button) = scene.view_labelled("Open link, Failed, show details") {
        scene.click(&button);
    }

    // Assert
    scene.check(
        scene
            .view_labelled("Open link, Failed, hide details")
            .is_some(),
        "failed receipt details can be disclosed",
    );
    scene.check(
        scene
            .accessibility()
            .matches("The page changed before the link opened.")
            .count()
            == 1,
        "the terminal notice remains readable after active status clears",
    );
    let transcript = panel
        .card_view()
        .subviews()
        .into_iter()
        .find_map(|view| view.downcast::<NSScrollView>().ok());
    scene.check(
        panel.card_view().bounds().size.height > compact_height,
        "expanding tool details grows the compact card when there is space",
    );
    scene.check(
        transcript.is_some_and(|scroll| {
            scroll
                .documentView()
                .is_some_and(|document| scroll.contentSize().height >= document.frame().size.height)
        }),
        "the entire current error is visible without scrolling",
    );
}

fn workspace_stopped(scene: &mut Scene) {
    // Arrange
    let panel = workspace(scene);
    panel.append_prompt("Read the guide");
    let row = panel.start_tool("Read page", "example.test/guide");
    panel.set_activity("Reading…", true);
    let weak = Rc::downgrade(&panel);
    panel.on_request(move |request| {
        if request == WorkspacePanelRequest::Stop
            && let Some(panel) = weak.upgrade()
        {
            panel.finish_tool(row, WorkspaceToolStatus::Cancelled, "Stopped by you");
            panel.set_activity("", false);
            panel.append_notice("Stopped", false);
        }
    });

    // Act
    if let Some(button) = scene.view_labelled("Stop agent") {
        scene.click(&button);
    }

    // Assert
    scene.check(
        scene
            .view_labelled("Read page, Cancelled, show details")
            .is_some(),
        "stopped receipts remain in the conversation",
    );
}

fn workspace_history(scene: &mut Scene) {
    use objc2_app_kit::{NSScrollView, NSTextView};
    use objc2_foundation::NSPoint;

    // Arrange
    let panel = workspace(scene);
    panel.append_answer(&"A paragraph from the guide.\n\n".repeat(80));
    let scroll = panel
        .card_view()
        .subviews()
        .into_iter()
        .find_map(|view| view.downcast::<NSScrollView>().ok());
    let Some(scroll) = scroll else {
        scene.check(false, "conversation scrolls");
        return;
    };
    let visible = scroll.contentView().documentVisibleRect();
    let bottom = visible.origin.y;
    scene.check(bottom > 0.0, "new rows follow the end initially");

    scene.host().layoutSubtreeIfNeeded();
    let location = scroll.convertPoint_toView(
        NSPoint::new(visible.size.width / 2.0, visible.size.height / 2.0),
        None,
    );
    let text = scene
        .host()
        .hitTest(location)
        .and_then(|view| view.downcast::<NSTextView>().ok());
    let (Some(text), Some(document)) = (text, scroll.documentView()) else {
        scene.check(
            false,
            "the pointer hits the answer text inside the document",
        );
        return;
    };
    scene.check(
        text.enclosingScrollView().as_deref() == Some(&*scroll),
        "answer text shares the transcript's native scroll view",
    );
    let character =
        text.characterIndexForInsertionAtPoint(text.convertPoint_fromView(location, None));
    scene.check(
        character > 0 && character < text.string().length(),
        "the text container lays out readable characters through the visible history",
    );

    // AppKit sends this scroll request to the closest clip view. The offscreen
    // window cannot receive a window-server wheel event.
    let destination =
        text.convertPoint_fromView(NSPoint::new(0.0, bottom - 120.0), Some(&document));
    text.scrollPoint(destination);
    let scrolled_up = scroll.contentView().documentVisibleRect().origin.y;
    scene.check(
        scrolled_up < bottom - 20.0,
        format!("scrolling over settled answer text moves from {bottom} to {scrolled_up}"),
    );
    let composer = scene
        .view_labelled("Message the agent")
        .map(|view| view.frame());

    // Act
    let original_size = panel.view().frame().size;
    let mut size = original_size;
    size.width -= 40.0;
    panel.view().setFrameSize(size);
    panel.view().setFrameSize(original_size);
    panel.append_answer("A follow-up arrived while you were reading above.");

    // Assert
    scene.check(
        (scroll.contentView().documentVisibleRect().origin.y - scrolled_up).abs() < 1.0,
        "new rows preserve a scrolled-up position",
    );
    scene.check_eq(
        scene
            .view_labelled("Message the agent")
            .map(|view| view.frame().origin.y),
        composer.map(|frame| frame.origin.y),
        "the composer remains anchored while history grows",
    );
}

fn workspace_follow_up(
    scene: &mut Scene,
    panel: &AgentWorkspacePanel,
    asked: &RefCell<Vec<WorkspacePanelRequest>>,
) {
    use objc2_app_kit::NSTextField;

    // Arrange
    let Some(field) = scene.view_labelled("Message the agent") else {
        scene.check(false, "composer is accessible");
        return;
    };
    panel.focus_composer();
    scene.type_text("Keep this draft");

    // Act
    scene.press(Key::Return);

    // Assert
    scene.check(
        asked.borrow().is_empty(),
        "Return keeps the next prompt unsent while running",
    );
    scene.check_eq(
        field
            .downcast_ref::<NSTextField>()
            .map(|field| field.stringValue().to_string()),
        Some("Keep this draft".to_owned()),
        "the unsent draft remains editable",
    );
}

fn workspace_citations(scene: &mut Scene) {
    // Arrange
    let panel = workspace(scene);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&asked);
    panel.on_request(move |request| log.borrow_mut().push(request));
    panel.append_answer_with_sources(
        "Sovereign power is **limited by law**. [99](quickie-cite:99)",
        &[
            WorkspaceCitation {
                id: 42,
                quote: "Sovereign power is limited by law".to_owned(),
            },
            WorkspaceCitation {
                id: 43,
                quote: "The rule remains accessible".to_owned(),
            },
        ],
    );
    panel.append_answer("The earlier source remains available after a follow-up.");

    // Act
    if let Some(chip) = scene.view_labelled("“Sovereign power is limited by law”") {
        scene.click(&chip);
    } else {
        scene.check(false, "the verified quote has an accessible citation chip");
    }

    // Assert
    scene.check_eq(
        asked.borrow().clone(),
        vec![WorkspacePanelRequest::Cite(42)],
        "only a click on a verified source requests its original identifier",
    );
    panel.expire_citation(42);
    if let Some(chip) = scene.view_labelled("Source unavailable: Sovereign power is limited by law")
    {
        scene.click(&chip);
    } else {
        scene.check(false, "a stale source is marked in place");
    }
    scene.check_eq(
        asked.borrow().len(),
        1,
        "a stale source cannot be opened again",
    );
    scene.check(
        scene
            .view_labelled("“The rule remains accessible”")
            .is_some(),
        "only the stale source is disabled",
    );
}

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
