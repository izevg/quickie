//! Stories of what floats up to say something or ask: the popovers, the sheets, and the
//! feedback group.

mod feedback;
mod popovers;
mod sheets;

use quickie_ui_shared::stories::Story;

pub const GROUPS: [&[Story]; 3] = [popovers::STORIES, sheets::STORIES, feedback::STORIES];
