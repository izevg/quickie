//! The agent at work (`docs/design/mocks/agent/session/`): the badge at the top of the page and
//! the frame around the window, both in the agent color, both there only while it acts.

pub mod badge;
pub mod frame;

pub use badge::AgentBadge;
pub use frame::AgentFrame;
