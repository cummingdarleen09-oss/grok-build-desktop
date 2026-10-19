//! Desktop agent runtime for the grok-build fork.
//!
//! This crate drives third-party providers from [`grok_providers`] and exposes
//! the session, tool, and settings surface the Tauri shell renders. The classic
//! grok TUI stays a separate CLI binary and is not embedded in the window.

#![forbid(unsafe_code)]

mod agent;
mod automations;
mod extensions;
mod mcp;
mod media;
mod mermaid;
mod sessions;
mod settings;
mod text;
mod tools;

pub use agent::{AgentEvent, TurnControl, run_turn};
pub use automations::{Automation, load_automations, save_automations};
pub use extensions::{ExtensionRecord, discover_extensions};
pub use grok_providers;
pub use mcp::McpPool;
pub use media::{MediaFile, list_media};
pub use mermaid::render_mermaid;
pub use sessions::{
    ChatMessage, ProjectGroup, Session, SessionSummary, StoredTool, branch_session,
    conversation_turns, create_session, delete_session, group_sessions, list_sessions,
    load_classic_session, load_session, normalize_project_path, rename_session, save_session,
};
pub use settings::{DesktopSettings, load_settings, save_settings};
pub use text::tr;
pub use tools::{ToolOutput, execute_tool, tool_defs};
