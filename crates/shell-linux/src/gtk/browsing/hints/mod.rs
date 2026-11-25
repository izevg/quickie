//! Native keyboard hints for the current page's visible links and controls.

mod script;
mod view;

use std::cell::{Cell, RefCell};
use std::ffi::{c_double, c_int, c_uint};
use std::ptr;
use std::rc::Rc;

use quickie_application::Followed;
use quickie_domain_browsing::{Focus, JarChoice};
use quickie_domain_commands::link_hint::{Typed, narrow};
use quickie_domain_commands::{Key, KeyStroke, Modifiers};
use quickie_engine_webkit::link_hints;

use crate::gtk::{App, connect, ffi};
use ffi::{Bool, Object};

const SCROLL_BOTH_AXES: c_int = 3;

#[derive(Default)]
pub(in crate::gtk) struct Hints {
    view: RefCell<Option<view::View>>,
    session: RefCell<Option<Session>>,
    generation: Cell<u64>,
    resize_idle: Cell<Option<c_uint>>,
    lifetime: Rc<()>,
}

struct Session {
    source: Object,
    viewport: view::Viewport,
    background: bool,
    phase: Phase,
}

enum Phase {
    Collecting,
    Labels { labels: Vec<String>, prefix: String },
    Following,
}

impl Hints {
    pub(in crate::gtk) fn attach(&self, overlay: Object, data: Object) {
        self.view.replace(Some(view::View::new(overlay)));
        connect(overlay, c"get-child-position", layout as *const _, data);
        // SAFETY: GTK is initialized and the overlay adopts both new controllers.
        let click = unsafe { gtk_gesture_click_new() };
        // SAFETY: This scroll controller observes both axes.
        let scroll = unsafe { gtk_event_controller_scroll_new(SCROLL_BOTH_AXES) };
        connect(click, c"pressed", pointer as *const _, data);
        connect(scroll, c"scroll", scrolled as *const _, data);
        for controller in [click, scroll] {
            // SAFETY: Both controllers were just created on the GTK thread.
            unsafe {
                ffi::gtk_event_controller_set_propagation_phase(controller, ffi::PHASE_CAPTURE);
            }
            // SAFETY: The live overlay adopts the controller on the GTK thread.
            unsafe { ffi::gtk_widget_add_controller(overlay, controller) };
        }
        // SAFETY: Browsing attaches the overlay after placing it in the browser window.
        let window = unsafe { ffi::gtk_widget_get_root(overlay) };
        if !window.is_null() {
            connect(
                window,
                c"notify::is-active",
                focus_changed as *const _,
                data,
            );
        }
    }

    pub(in crate::gtk) fn hide(&self) {
        if let Some(id) = self.resize_idle.take() {
            // SAFETY: The GTK thread still owns this registered source.
            unsafe { ffi::g_source_remove(id) };
        }
        if self.session.borrow_mut().take().is_some() {
            self.generation.set(self.generation.get().wrapping_add(1));
            if let Some(view) = self.view.borrow_mut().as_mut() {
                view.hide();
            }
        }
    }

    fn viewport(&self, source: Object) -> Option<view::Viewport> {
        self.view.borrow().as_ref()?.viewport(source)
    }
}

pub(in crate::gtk) fn active(app: &App) -> bool {
    app.browsing
        .hints
        .session
        .borrow()
        .as_ref()
        .is_some_and(|session| !matches!(session.phase, Phase::Following))
}

pub(in crate::gtk) fn hide(app: &App) {
    app.browsing.hints.hide();
}

pub(in crate::gtk) fn toggle(app: &App, background: bool) {
    let hints = &app.browsing.hints;
    if active(app) {
        hints.hide();
        return;
    }
    let Some(widgets) = app.widgets.get() else {
        return;
    };
    hints.hide();
    let Some(viewport) = hints.viewport(widgets.view) else {
        return;
    };
    hints.session.replace(Some(Session {
        source: widgets.view,
        viewport,
        phase: Phase::Collecting,
        background,
    }));
    issue(app, widgets.view, link_hints::COLLECT_SCRIPT, collected);
}

fn issue(app: &App, source: Object, text: &str, done: fn(&App, Object, &str)) {
    let hints = &app.browsing.hints;
    let generation = hints.generation.get();
    let lifetime = Rc::downgrade(&hints.lifetime);
    let app_pointer = ptr::from_ref(app);
    if !script::evaluate(source, text, move |view, answer| {
        if lifetime.upgrade().is_none() {
            return;
        }
        // SAFETY: The app owns the lifetime token and WebKit calls back on the GTK thread.
        let app = unsafe { &*app_pointer };
        if app.browsing.hints.generation.get() == generation {
            done(app, view, answer);
        }
    }) {
        hints.hide();
    }
}

fn collected(app: &App, source: Object, answer: &str) {
    let hints = &app.browsing.hints;
    let mut session = hints.session.borrow_mut();
    let Some(current) = session.as_mut() else {
        return;
    };
    if current.source != source || !matches!(current.phase, Phase::Collecting) {
        return;
    }
    if hints.resize_idle.get().is_some() || hints.viewport(source) != Some(current.viewport) {
        drop(session);
        hints.hide();
        return;
    }
    let mut view = hints.view.borrow_mut();
    let shown = view
        .as_mut()
        .and_then(|view| view.show(current.viewport, answer, current.background));
    if let Some(labels) = shown {
        current.phase = Phase::Labels {
            labels,
            prefix: String::new(),
        };
    } else {
        session.take();
    }
}

pub(in crate::gtk) fn key(app: &App, stroke: KeyStroke) -> bool {
    if !active(app) || stroke.modifiers.makes_shortcut() {
        return false;
    }
    if stroke.key == Key::Escape {
        hide(app);
        return true;
    }
    let hints = &app.browsing.hints;
    let mut session = hints.session.borrow_mut();
    let Some(current) = session.as_mut() else {
        return false;
    };
    if hints.resize_idle.get().is_some() || hints.viewport(current.source) != Some(current.viewport)
    {
        drop(session);
        hints.hide();
        return true;
    }
    let Phase::Labels { labels, prefix } = &mut current.phase else {
        return matches!(stroke.key, Key::Letter(_) | Key::Backspace);
    };
    match stroke.key {
        Key::Backspace => {
            prefix.pop();
        }
        Key::Letter(letter) => {
            prefix.push(letter.as_char());
            match narrow(labels, prefix) {
                Typed::Miss => {
                    prefix.pop();
                    return true;
                }
                Typed::Narrowed => {}
                Typed::Hit(index) => {
                    let source = current.source;
                    let background = current.background || stroke.modifiers == Modifiers::SHIFT;
                    current.phase = Phase::Following;
                    drop(session);
                    if let Some(view) = hints.view.borrow_mut().as_mut() {
                        view.hide();
                    }
                    issue(
                        app,
                        source,
                        &link_hints::follow_script(index, background),
                        followed,
                    );
                    return true;
                }
            }
        }
        _ => return true,
    }
    if let Some(view) = hints.view.borrow().as_ref() {
        view.restyle(labels, prefix);
    }
    true
}

fn followed(app: &App, source: Object, answer: &str) {
    let browsing = &app.browsing;
    browsing.hints.hide();
    let (url, focus) = match link_hints::followed(answer) {
        Followed::Done => return,
        Followed::Background(url) => (url, Focus::Background),
        Followed::NewTab(url) => (url, Focus::Foreground),
    };
    if let (Some(opener), Some(window)) = (browsing.pages.tab(source), browsing.window.get()) {
        let opened =
            browsing.run(|state| state.open(window, url, JarChoice::SameAs(opener), focus));
        browsing.settle(app);
        if opened.is_some() && focus == Focus::Background {
            super::notify(
                app,
                super::toasts::Toast::info("Opened in a background tab"),
            );
        }
    }
}

pub(in crate::gtk) fn observe(view: Object, data: Object) {
    script::observe(view);
    connect(
        view,
        c"web-process-terminated",
        terminated as *const _,
        data,
    );
}

pub(in crate::gtk) fn cancel_for(app: &App, view: Object) {
    let matches = app
        .browsing
        .hints
        .session
        .borrow()
        .as_ref()
        .is_some_and(|session| session.source == view);
    if matches {
        hide(app);
    }
}

extern "C" fn layout(_: Object, child: Object, _: Object, data: Object) -> Bool {
    // SAFETY: The live overlay emits this signal using the App registered in attach.
    let app = unsafe { &*data.cast::<App>() };
    let hints = &app.browsing.hints;
    if hints.resize_idle.get().is_none()
        && let Some(view) = hints.view.borrow().as_ref()
        && view.is_layer(child)
        && let Some(session) = hints.session.borrow().as_ref()
        && !matches!(session.phase, Phase::Following)
        && view.viewport(session.source) != Some(session.viewport)
    {
        // The overlay is allocating its children. Defer removing badges until it finishes.
        // SAFETY: GTK keeps the App alive on this thread until detach cancels the source.
        hints
            .resize_idle
            .set(Some(unsafe { g_idle_add(clear_after_resize, data) }));
    }
    ffi::FALSE
}

extern "C" fn clear_after_resize(data: Object) -> Bool {
    // SAFETY: The live App owns the registered idle callback and runs it on the GTK thread.
    let app = unsafe { &*data.cast::<App>() };
    app.browsing.hints.resize_idle.set(None);
    hide(app);
    ffi::FALSE
}

extern "C" fn terminated(view: Object, _: c_int, data: Object) {
    // SAFETY: The attached WebKit signal supplies the live GTK App from observe.
    let app = unsafe { &*data.cast::<App>() };
    cancel_for(app, view);
}

extern "C" fn focus_changed(window: Object, _: Object, data: Object) {
    // SAFETY: This signal belongs to the live browser window with the App callback data.
    if unsafe { gtk_window_is_active(window) } == ffi::FALSE {
        dismiss(data);
    }
}

extern "C" fn pointer(_: Object, _: c_int, _: c_double, _: c_double, data: Object) {
    dismiss(data);
}

extern "C" fn scrolled(_: Object, _: c_double, _: c_double, data: Object) -> Bool {
    dismiss(data);
    ffi::FALSE
}

fn dismiss(data: Object) {
    // SAFETY: The registered signal callbacks supply the live App passed to attach.
    let app = unsafe { &*data.cast::<App>() };
    hide(app);
}

#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gtk_gesture_click_new() -> Object;
    fn gtk_event_controller_scroll_new(flags: c_int) -> Object;
    fn gtk_window_is_active(window: Object) -> Bool;
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_idle_add(callback: extern "C" fn(Object) -> Bool, data: Object) -> c_uint;
}
