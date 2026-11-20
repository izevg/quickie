//! Stories of what the user types into and steers through: the palette group, the tabs group
//! with the mini player, and the page group, with the page's frame in the window.

mod frame;
mod page;
mod palette;
mod tabs;

use quickie_ui_shared::stories::Story;

pub const GROUPS: [&[Story]; 6] = [
    palette::STORIES,
    palette::address::STORIES,
    tabs::STORIES,
    tabs::player::STORIES,
    page::STORIES,
    frame::STORIES,
];
