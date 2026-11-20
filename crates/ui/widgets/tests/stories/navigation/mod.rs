//! Stories of what the user types into and steers through: the palette group, the tabs group
//! with the mini player, and the page group.

mod page;
mod palette;
mod player;
mod tabs;

use quickie_ui_shared::stories::Story;

pub const GROUPS: [&[Story]; 5] = [
    palette::STORIES,
    palette::address::STORIES,
    tabs::STORIES,
    player::STORIES,
    page::STORIES,
];
