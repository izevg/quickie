//! Stories of what the user types into and steers through: the palette group, the tabs group,
//! and the page group.

mod page;
mod palette;
mod tabs;

use quickie_ui_shared::stories::Story;

pub const GROUPS: [&[Story]; 4] = [
    palette::STORIES,
    palette::address::STORIES,
    tabs::STORIES,
    page::STORIES,
];
