//! FSD `widgets` layer of Quickie's UI: the composed blocks that float above the page.
//!
//! Slices are grouped by meaning (`docs/design/inventory.md`, widgets): `palette`, `tabs`,
//! `feedback`, `page`, `hints`, `popovers`, `sheets`, `agent`. A slice never uses another, not
//! even one in its own group; what two share moves down to `quickie-ui-shared`.

#![expect(
    unsafe_code,
    reason = "AppKit through objc2: field selection and superviews, the address URL's \
              paragraph style, the find bar's buttons and edits, Core Animation transactions, the \
              toast and chip timers, the overview's click target and edit observer, layer \
              contents, and the answer's text storage and link delegate"
)]

pub mod agent;
pub mod feedback;
pub mod hints;
pub mod page;
pub mod palette;
pub mod popovers;
pub mod sheets;
pub mod tabs;

pub use agent::agent_badge::AgentBadge;
pub use agent::agent_settings::{
    AgentSettings, SettingsForm, SettingsRequest, SettingsSummary, Tone,
};
pub use agent::answer_panel::wording::{self as agent_wording, Activity};
pub use agent::answer_panel::{AgentAnswerPanel, AnswerHeader, PanelRequest};
pub use feedback::fullscreen_notice::FullscreenNotice;
pub use feedback::key_log::KeyLogPanel;
pub use feedback::toast_stack::ToastStack;
pub use hints::link_hints_layer::{HintKey, LinkHintsLayer};
pub use page::find_bar::{FindBar, FindRequest};
pub use page::load_progress_line::LoadProgressLine;
pub use page::origin_chip::OriginChip;
pub use palette::address_overlay::AddressOverlay;
pub use palette::command_palette::CommandPalette;
pub use popovers::downloads_popover::{DownloadsPopover, DownloadsRequest};
pub use popovers::extension_popover::ExtensionPopover;
pub use popovers::shields_popover::{ShieldsPopover, ShieldsRequest, ShieldsSummary, SiteToggle};
pub use sheets::dialog_sheet::{DialogChoice, DialogKind, DialogRequest, DialogSheet};
pub use sheets::extension_sheet::{ExtensionRequest, ExtensionSheet, Moment, describe};
pub use sheets::permission_sheet::PermissionSheet;
pub use tabs::tab_overview::grid::Move;
pub use tabs::tab_overview::pinch::Step as PinchStep;
pub use tabs::tab_overview::{OverviewTab, TabOverview};
pub use tabs::tab_switcher::TabSwitcher;
