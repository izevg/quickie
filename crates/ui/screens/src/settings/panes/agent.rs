//! Agent: the agent settings form (`widgets/agent`, `agent_settings`) on a page of its own,
//! under a heading. The browser owns the form and answers its buttons; the pane only fits it
//! to the column.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSTextField, NSView};
use objc2_foundation::NSPoint;
use quickie_ui_widgets::AgentSettings;

use crate::settings::chrome::{Column, INNER, ScrollPage, heading};

pub struct Agent {
    page: ScrollPage,
    heading: Retained<NSTextField>,
    form: Retained<NSView>,
}

impl Agent {
    pub fn new(form: &AgentSettings, mtm: MainThreadMarker) -> Self {
        let agent = Self {
            page: ScrollPage::new("Agent", mtm),
            heading: heading("Endpoint", mtm),
            form: Retained::from(form.view()),
        };
        agent.page.column().addSubview(&agent.heading);
        agent.page.column().addSubview(&agent.form);
        agent.fit(form);
        agent
    }

    pub fn view(&self) -> &NSView {
        self.page.view()
    }

    /// Fits the form to the column again: its status line may have changed its height.
    pub fn fit(&self, form: &AgentSettings) {
        form.fit_width(INNER);
        let mut column = Column::new();
        column.heading(&self.heading);
        self.form
            .setFrameOrigin(NSPoint::new(column.origin().x, column.top));
        column.gap(self.form.frame().size.height);
        self.page.set_content_height(column.top);
    }
}
