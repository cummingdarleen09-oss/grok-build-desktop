//! Desktop agent runtime for the grok-build fork.
//!
//! This crate drives third-party providers from [`grok_providers`] and exposes
//! the session, tool, and settings surface the Tauri shell renders. The classic
//! grok TUI stays a separate binary; the desktop app can host it in a PTY.

#![forbid(unsafe_code)]

mod agent;
mod extensions;
mod sessions;
mod settings;
mod text;
mod tools;

pub use agent::{AgentEvent, TurnControl, run_turn};
pub use extensions::{ExtensionRecord, discover_extensions};
pub use grok_providers;
pub use sessions::{
    ChatMessage, Session, SessionSummary, StoredTool, create_session, delete_session,
    list_sessions, load_session, save_session,
};
pub use settings::{DesktopSettings, load_settings, save_settings};
pub use text::tr;
pub use tools::{ToolOutput, execute_tool, tool_defs};
