//! An explicit, persistent GTK workspace bound to one source `WebKit` view.

pub(crate) mod automation;
mod read;
mod service;
mod turn;
mod view;

use std::cell::{Cell, RefCell};
use std::ffi::{c_int, c_uint, c_ulong};
use std::ptr;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::Builder;

use quickie_domain_agents::{ApiKey, BaseUrl, LlmEndpoint, ModelId, Question};

use super::{App, Widgets, connect, ffi, keys};
use ffi::{Bool, Object};
use service::{Saved, Settings};
use turn::{ProviderWorkspace, Turn};
pub(super) use view::View;

const LOAD_STARTED: c_int = 0;
const POLL_MILLIS: c_uint = 35;
const SOURCE_CHANGED: &str = "The source page changed. Ask again to read the new page.";
type SettingsResult = Result<Saved, String>;

#[derive(Default)]
pub(super) struct Agent {
    source: RefCell<Option<Source>>,
    revision: Cell<u64>,
    endpoint: RefCell<Option<LlmEndpoint>>,
    model: RefCell<ProviderWorkspace>,
    queued: RefCell<Option<QueuedAsk>>,
    settings_open: Cell<bool>,
    job: RefCell<Option<Receiver<SettingsResult>>>,
    turn: RefCell<Option<Turn>>,
    timer: Cell<c_uint>,
}

struct QueuedAsk {
    question: Question,
    revision: u64,
}

struct Source {
    view: Object,
    changed: c_ulong,
    loading: c_ulong,
}

impl Source {
    fn bind(view: Object, app: &App) -> Self {
        let data = app_data(app);
        // SAFETY: Retain the live source independently of the active tab.
        unsafe { g_object_ref(view) };
        // SAFETY: This is the live WebKit view and the App outlives the URI handler.
        let changed = unsafe {
            ffi::g_signal_connect_data(
                view,
                c"notify::uri".as_ptr(),
                uri_changed as *const _,
                data,
                None,
                0,
            )
        };
        // SAFETY: This is the live WebKit view and the App outlives the load handler.
        let loading = unsafe {
            ffi::g_signal_connect_data(
                view,
                c"load-changed".as_ptr(),
                load_changed as *const _,
                data,
                None,
                0,
            )
        };
        Self {
            view,
            changed,
            loading,
        }
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        // SAFETY: We retained the view and registered this URI handler.
        unsafe { g_signal_handler_disconnect(self.view, self.changed) };
        // SAFETY: We retained the view and registered this load handler.
        unsafe { g_signal_handler_disconnect(self.view, self.loading) };
        // SAFETY: This releases our reference after disconnecting its handlers.
        unsafe { ffi::g_object_unref(self.view) };
    }
}

impl Agent {
    pub(super) fn shutdown(&self) {
        let timer = self.timer.replace(0);
        if timer != 0 {
            // SAFETY: This removes the active source before the App backing its data drops.
            unsafe { ffi::g_source_remove(timer) };
        }
        self.source.take();
        self.turn.take();
        self.job.take();
    }

    fn source(&self) -> Option<Object> {
        self.source.borrow().as_ref().map(|source| source.view)
    }

    fn configured(&self) -> bool {
        self.endpoint.borrow().is_some()
    }

    fn wait(&self, app: &App) {
        if self.timer.get() == 0 {
            // SAFETY: The App remains live during GTK dispatch and shutdown removes this source.
            self.timer
                .set(unsafe { ffi::g_timeout_add(POLL_MILLIS, poll, app_data(app)) });
        }
    }
}

pub(super) fn attach(parent: Object, _web_view: Object, data: Object) -> View {
    let view = View::new(parent);
    for (widget, callback) in [
        (view.submit, submit as *const _),
        (view.stop, stop as *const _),
        (view.close, close_clicked as *const _),
        (view.settings, settings_clicked as *const _),
        (view.setup, settings_clicked as *const _),
        (view.form.back, back as *const _),
        (view.form.save, save as *const _),
        (view.form.remove, remove_key as *const _),
    ] {
        connect(widget, c"clicked", callback, data);
    }
    connect(view.composer, c"activate", submit as *const _, data);
    view
}

pub(super) fn open(app: &App, widgets: &Widgets) {
    let agent = &app.agent;
    if agent.source().is_none() {
        agent.source.replace(Some(Source::bind(widgets.view, app)));
        refresh_source(widgets.agent, widgets.view);
        widgets
            .agent
            .activity("Loading agent settings…", false, false);
        start_settings(app, widgets, Settings::Load);
    }
    widgets.agent.show(true);
    if !agent.settings_open.get() {
        widgets.agent.focus();
    }
}

pub(super) fn settings(app: &App, widgets: &Widgets) {
    open(app, widgets);
    if !app.agent.settings_open.replace(true) {
        sync_form(app, widgets.agent);
        widgets.agent.show_settings(true);
    }
}

pub(super) fn ask(app: &App, widgets: &Widgets, text: &str) {
    open(app, widgets);
    if app.agent.source() != Some(widgets.view) {
        widgets.agent.row(
            "Close the Agent workspace to ask about the active tab.",
            c"agent-problem",
        );
        return;
    }
    match Question::try_from(text) {
        Ok(question) => {
            let settings_pending = app.agent.job.borrow().is_some();
            if settings_pending && let Err(error) = read::source(widgets.view) {
                widgets.agent.row(&error, c"agent-problem");
                return;
            }
            let revision = app.agent.revision.get();
            widgets.agent.set_draft(question.as_str());
            if settings_pending {
                app.agent
                    .queued
                    .replace(Some(QueuedAsk { question, revision }));
            } else {
                submit_question(app, widgets, question);
            }
        }
        Err(error) => {
            widgets.agent.row(&error.to_string(), c"agent-problem");
        }
    }
}

pub(super) fn key_pressed(app: &App, widgets: &Widgets, key: c_uint, mods: c_uint) -> bool {
    if mods & keys::SHORTCUT_MODIFIERS != 0
        || key != ffi::KEY_ESCAPE
        || !view::widget_focused(widgets.agent.panel)
    {
        return false;
    }
    if app.agent.settings_open.get() {
        close_settings(app, widgets.agent);
    } else if app.agent.turn.borrow().is_some() {
        stop_turn(app, widgets.agent, "Stopped");
    } else {
        close(app, widgets);
    }
    true
}

pub(super) fn key_panel_open(widgets: &Widgets) -> bool {
    // SAFETY: The live panel stays attached to the browser host on its owning GTK thread.
    let visible = unsafe { gtk_widget_get_visible(widgets.agent.panel) };
    visible != ffi::FALSE
}

pub(super) fn source_removed(app: &App, source: Object) {
    if app.agent.source() == Some(source)
        && let Some(widgets) = app.widgets.get()
    {
        close(app, &widgets);
    }
}

fn close(app: &App, widgets: &Widgets) {
    app.agent.turn.take();
    app.agent.model.take();
    app.agent.queued.take();
    app.agent.source.take();
    close_settings(app, widgets.agent);
    widgets.agent.set_draft("");
    widgets.agent.clear();
    widgets.agent.show(false);
    // SAFETY: The still-live active browser view belongs to the same window.
    unsafe { ffi::gtk_widget_grab_focus(widgets.view) };
}

fn close_settings(app: &App, panel: View) {
    app.agent.settings_open.set(false);
    panel.show_settings(false);
}

fn stop_turn(app: &App, panel: View, message: &str) {
    if let Some(turn) = app.agent.turn.take() {
        app.agent.model.borrow_mut().cancel();
        let focused = view::widget_focused(panel.panel);
        turn.stop(panel, message);
        if focused && !app.agent.settings_open.get() {
            panel.focus();
        }
    }
}

fn submit_question(app: &App, widgets: &Widgets, question: Question) {
    let agent = &app.agent;
    if agent.turn.borrow().is_some() || agent.job.borrow().is_some() {
        return;
    }
    let Some(source) = agent.source() else { return };
    let Some(endpoint) = agent.endpoint.borrow().clone() else {
        widgets
            .agent
            .activity("Set up an endpoint to ask about this page", false, false);
        return;
    };
    match Turn::start(
        app,
        source,
        question,
        endpoint,
        agent.revision.get(),
        widgets.agent,
    ) {
        Ok(turn) => {
            agent.turn.replace(Some(turn));
            widgets.agent.set_draft("");
            agent.wait(app);
        }
        Err(error) => {
            widgets.agent.row(&error, c"agent-problem");
        }
    }
}

fn start_settings(app: &App, widgets: &Widgets, intent: Settings) {
    let agent = &app.agent;
    if agent.job.borrow().is_some() {
        return;
    }
    let (sender, receiver) = mpsc::channel();
    let started = Builder::new()
        .name("quickie-agent-settings".into())
        .spawn(move || {
            let _ = sender.send(service::configure(intent));
        });
    if started.is_err() {
        widgets
            .agent
            .form
            .status("Could not open the agent settings", false);
        widgets.agent.activity(
            "Could not open the agent settings",
            false,
            agent.configured(),
        );
        return;
    }
    agent.job.replace(Some(receiver));
    widgets.agent.form.status("Opening agent settings…", true);
    agent.wait(app);
}

fn sync_form(app: &App, panel: View) {
    let endpoint = app.agent.endpoint.borrow();
    if let Some(endpoint) = endpoint.as_ref() {
        panel.form.set(
            endpoint.base().as_str(),
            endpoint.model().as_str(),
            endpoint.key().is_some(),
        );
    } else {
        panel.form.set("", "", false);
    }
}

fn poll_settings(app: &App, widgets: &Widgets) {
    let agent = &app.agent;
    let result = match agent.job.borrow().as_ref().map(Receiver::try_recv) {
        Some(Ok(result)) => result,
        Some(Err(TryRecvError::Disconnected)) => Err("Agent settings ended unexpectedly".into()),
        Some(Err(TryRecvError::Empty)) | None => return,
    };
    agent.job.take();
    let status = match result {
        Ok(saved) => {
            agent.model.borrow_mut().bind(saved.endpoint.as_ref());
            agent.endpoint.replace(saved.endpoint);
            sync_form(app, widgets.agent);
            saved.status
        }
        Err(error) => error,
    };
    widgets.agent.form.status(&status, false);
    if agent.source().is_none() {
        return;
    }
    let message = if agent.configured() {
        ""
    } else {
        "Set up an endpoint to ask about this page"
    };
    widgets.agent.activity(message, false, agent.configured());
    if agent.settings_open.get() {
        widgets.agent.form.focus();
    } else if view::widget_focused(widgets.agent.panel) || agent.queued.borrow().is_some() {
        widgets.agent.focus();
    }
    let queued = agent.queued.take();
    if let Some(queued) = queued.filter(|queued| queued.revision == agent.revision.get()) {
        submit_question(app, widgets, queued.question);
    }
}

extern "C" fn poll(data: Object) -> Bool {
    with_widgets(data, |app, widgets| {
        poll_settings(app, widgets);
        let agent = &app.agent;
        if let Some(turn) = agent.turn.take() {
            agent.turn.replace(turn.poll(app, widgets.agent));
        }
        if agent.job.borrow().is_some() || agent.turn.borrow().is_some() {
            return ffi::TRUE;
        }
        agent.timer.set(0);
        ffi::FALSE
    })
    .unwrap_or(ffi::FALSE)
}

extern "C" fn submit(_: Object, data: Object) {
    with_widgets(data, |app, widgets| {
        match Question::try_from(widgets.agent.draft().as_str()) {
            Ok(question) => submit_question(app, widgets, question),
            Err(error) => {
                widgets.agent.row(&error.to_string(), c"agent-problem");
            }
        }
    });
}

extern "C" fn stop(_: Object, data: Object) {
    with_widgets(data, |app, widgets| {
        stop_turn(app, widgets.agent, "Stopped");
    });
}

extern "C" fn close_clicked(_: Object, data: Object) {
    with_widgets(data, close);
}

extern "C" fn settings_clicked(_: Object, data: Object) {
    with_widgets(data, settings);
}

extern "C" fn back(_: Object, data: Object) {
    with_widgets(data, |app, widgets| close_settings(app, widgets.agent));
}

extern "C" fn save(_: Object, data: Object) {
    with_widgets(data, |app, widgets| {
        let form = widgets.agent.form;
        let (base, model, key) = form.values();
        let parsed = parse_settings(&base, &model, &key);
        form.clear_key();
        match parsed {
            Ok(intent) => {
                stop_turn(app, widgets.agent, "Stopped before changing the endpoint");
                start_settings(app, widgets, intent);
            }
            Err(error) => form.status(&error, false),
        }
    });
}

fn parse_settings(base: &str, model: &str, key: &str) -> Result<Settings, String> {
    let base = BaseUrl::try_from(base).map_err(|error| format!("Endpoint: {error}"))?;
    let model = ModelId::try_from(model).map_err(|error| format!("Model: {error}"))?;
    let key = if key.trim().is_empty() {
        None
    } else {
        Some(ApiKey::try_from(key).map_err(|error| error.to_string())?)
    };
    Ok(Settings::Save(base, model, key))
}

extern "C" fn remove_key(_: Object, data: Object) {
    with_widgets(data, |app, widgets| {
        stop_turn(app, widgets.agent, "Stopped before changing credentials");
        start_settings(app, widgets, Settings::RemoveKey);
    });
}

extern "C" fn load_changed(source: Object, event: c_int, data: Object) {
    if event == LOAD_STARTED {
        document_changed(source, data);
    }
}

extern "C" fn uri_changed(source: Object, _: Object, data: Object) {
    document_changed(source, data);
}

fn document_changed(source: Object, data: Object) {
    with_widgets(data, |app, widgets| {
        if app.agent.source() != Some(source) {
            return;
        }
        let revision = &app.agent.revision;
        revision.set(revision.get().wrapping_add(1));
        if let Some(cancelled) = app.agent.queued.take() {
            if widgets.agent.draft() == cancelled.question.as_str() {
                widgets.agent.set_draft("");
            }
            widgets.agent.row(SOURCE_CHANGED, c"agent-source");
        }
        if app
            .agent
            .turn
            .borrow()
            .as_ref()
            .is_some_and(Turn::needs_current_document)
        {
            stop_turn(app, widgets.agent, SOURCE_CHANGED);
        }
        refresh_source(widgets.agent, source);
    });
}

fn refresh_source(panel: View, source: Object) {
    let url = read::location(source);
    panel.source(
        url.as_ref()
            .and_then(url::Url::host_str)
            .unwrap_or("Current page"),
    );
}

fn app_data(app: &App) -> Object {
    ptr::from_ref(app).cast_mut().cast()
}

fn with_widgets<T>(data: Object, work: impl FnOnce(&App, &Widgets) -> T) -> Option<T> {
    // SAFETY: All agent signals and timers use the live App from the main GTK dispatch loop.
    let app = unsafe { &*data.cast::<App>() };
    app.widgets.get().as_ref().map(|widgets| work(app, widgets))
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_widget_get_visible(widget: Object) -> Bool;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_ref(object: Object) -> Object;
    fn g_signal_handler_disconnect(object: Object, id: c_ulong);
}
