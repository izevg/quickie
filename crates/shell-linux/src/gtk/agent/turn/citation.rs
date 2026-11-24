//! Reveals verified quotes only in the exact source document that supplied them.

use quickie_domain_agents::Citation;

use super::super::{View, app_data, with_widgets};
use crate::gtk::ffi::{self, Object};
use crate::gtk::{App, find};

struct Target {
    app: Object,
    revision: u64,
    anchor: String,
}

pub(super) fn show(app: &App, panel: View, revision: u64, citation: Citation) {
    let button = panel.source_chip(&citation.quote);
    let target = Box::new(Target {
        app: app_data(app),
        revision,
        anchor: citation.anchor,
    });
    // SAFETY: The live button owns its typed callback data through the destroy notifier.
    unsafe {
        ffi::g_signal_connect_data(
            button,
            c"clicked".as_ptr(),
            reveal as *const _,
            Box::into_raw(target).cast(),
            Some(drop_target),
            0,
        );
    }
}

extern "C" fn reveal(button: Object, data: Object) {
    // SAFETY: The button's signal keeps this Target alive through its callback.
    let target = unsafe { &*data.cast::<Target>() };
    with_widgets(target.app, |app, widgets| {
        let agent = &app.agent;
        if agent.revision.get() != target.revision || agent.source().is_none() {
            // SAFETY: The live button copies this static label.
            unsafe { ffi::gtk_button_set_label(button, c"Source unavailable".as_ptr()) };
            // SAFETY: The live button can be disabled in its owning GTK signal.
            unsafe { ffi::gtk_widget_set_sensitive(button, ffi::FALSE) };
        } else if agent.source() != Some(widgets.view) {
            widgets.agent.row(
                "Switch to the source tab to reveal this quote.",
                c"agent-source",
            );
        } else {
            find::show_query(app, widgets, &target.anchor);
        }
    });
}

unsafe extern "C" fn drop_target(data: Object, _: Object) {
    // SAFETY: The signal releases the one boxed Target in its destroy notifier.
    drop(unsafe { Box::from_raw(data.cast::<Target>()) });
}
