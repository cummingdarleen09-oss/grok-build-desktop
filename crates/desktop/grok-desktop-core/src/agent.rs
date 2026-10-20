use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::StreamExt;
use grok_providers::{ProviderClient, ProviderConfig, ProviderEvent, StopReason, Turn};
use serde::Serialize;

use crate::mcp::McpPool;
use crate::sessions::{ChatMessage, Session, StoredTool, conversation_turns, touch};
use crate::text::tr;
use crate::tools::{ToolOutput, execute_tool, needs_approval, tool_defs};

const MAX_ROUNDS: usize = 6;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ApprovalNeeded {
        id: String,
        name: String,
        arguments: String,
    },
    ToolStarted {
        id: String,
        name: String,
        arguments: String,
    },
    ToolFinished {
        id: String,
        output: String,
        is_error: bool,
    },
    Error {
        message: String,
    },
    /// Progress the user should see while no text is streaming yet.
    Status {
        text: String,
    },
    Done,
}

pub struct TurnControl {
    pub cancel: Arc<AtomicBool>,
}

type EventStream = Pin<Box<dyn futures_util::Stream<Item = Result<ProviderEvent, String>> + Send>>;

#[allow(clippy::too_many_arguments)]
pub async fn run_turn<F, Fut>(
    session: &mut Session,
    provider: &ProviderConfig,
    api_key: &str,
    user_text: &str,
    lang: &str,
    ask: bool,
    plan: bool,
    disabled_tools: &[String],
    client: &ProviderClient,
    control: &TurnControl,
    mcp: &mut McpPool,
    mut on_event: impl FnMut(AgentEvent),
    mut approve: F,
) -> Result<(), String>
where
    F: FnMut(String, String, String) -> Fut,
    Fut: Future<Output = bool>,
{
    let mut tools = tool_defs(lang, disabled_tools);
    if plan {
        tools.retain(|tool| {
            !matches!(
                tool.name.as_str(),
                "write_file" | "search_replace" | "run_terminal"
            )
        });
    } else {
        tools.extend(mcp.tool_defs());
    }
    let user = ChatMessage {
        id: new_id(),
        role: "user".into(),
        text: user_text.to_string(),
        reasoning: String::new(),
        tool_calls: Vec::new(),
    };
    if session.messages.is_empty() {
        session.title = title_from(user_text, lang);
    }
    session.provider_id = provider.id.clone();
    session.model = provider.model.clone();
    session.messages.push(user);
    touch(session);

    let open_stream = |turns: &[Turn]| {
        let turns = turns.to_vec();
        let tools = tools.clone();
        async move {
            if api_key.trim().is_empty() {
                return Err(tr(lang, "missing_key"));
            }
            let stream = client
                .stream(grok_providers::CompletionRequest {
                    provider,
                    api_key,
                    turns: &turns,
                    tools: &tools,
                })
                .await
                .map_err(|error| error.detailed())?;
            let boxed: EventStream =
                Box::pin(stream.map(|event| event.map_err(|error| error.detailed())));
            Ok(boxed)
        }
    };

    drive(
        session,
        lang,
        plan,
        ask,
        &tools,
        mcp,
        control,
        &mut on_event,
        &mut approve,
        open_stream,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn drive<F, Fut, Open, OpenFut>(
    session: &mut Session,
    lang: &str,
    plan: bool,
    ask: bool,
    tools: &[grok_providers::ToolDef],
    mcp: &mut McpPool,
    control: &TurnControl,
    on_event: &mut impl FnMut(AgentEvent),
    approve: &mut F,
    mut open_stream: Open,
) -> Result<(), String>
where
    F: FnMut(String, String, String) -> Fut,
    Fut: Future<Output = bool>,
    Open: FnMut(&[Turn]) -> OpenFut,
    OpenFut: Future<Output = Result<EventStream, String>>,
{
    let mut exhausted = false;
    for round in 0..MAX_ROUNDS {
        if control.cancel.load(Ordering::Relaxed) {
            on_event(AgentEvent::Error {
                message: tr(lang, "cancelled"),
            });
            on_event(AgentEvent::Done);
            return Ok(());
        }
        let turns = transcript(session, lang, plan);
        let mut stream = open_stream(&turns).await?;
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut calls: Vec<StoredTool> = Vec::new();
        let mut stop = None;
        let mut failure = None;
        while let Some(event) = stream.next().await {
            if control.cancel.load(Ordering::Relaxed) {
                keep_partial(session, &mut text, &mut reasoning);
                on_event(AgentEvent::Error {
                    message: tr(lang, "cancelled"),
                });
                on_event(AgentEvent::Done);
                return Ok(());
            }
            let event = match event {
                Ok(event) => event,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            };
            match event {
                ProviderEvent::Text { text: delta } => {
                    text.push_str(&delta);
                    on_event(AgentEvent::TextDelta { text: delta });
                }
                ProviderEvent::Reasoning { text: delta } => {
                    reasoning.push_str(&delta);
                    on_event(AgentEvent::ReasoningDelta { text: delta });
                }
                ProviderEvent::ToolStart { index, id, name } => {
                    let slot = index as usize;
                    if calls.len() <= slot {
                        calls.resize(
                            slot + 1,
                            StoredTool {
                                id: String::new(),
                                name: String::new(),
                                arguments: String::new(),
                                output: None,
                                is_error: false,
                            },
                        );
                    }
                    if !id.is_empty() {
                        calls[slot].id = id;
                    }
                    if !name.is_empty() {
                        calls[slot].name = name;
                    }
                }
                ProviderEvent::ToolArgs { index, fragment } => {
                    let slot = index as usize;
                    if calls.len() <= slot {
                        calls.resize(
                            slot + 1,
                            StoredTool {
                                id: String::new(),
                                name: String::new(),
                                arguments: String::new(),
                                output: None,
                                is_error: false,
                            },
                        );
                    }
                    calls[slot].arguments.push_str(&fragment);
                }
                ProviderEvent::Usage { .. } => {}
                ProviderEvent::Stop { reason } => stop = Some(reason),
            }
        }
        if let Some(error) = failure {
            keep_partial(session, &mut text, &mut reasoning);
            return Err(error);
        }
        calls.retain(|call| !call.name.is_empty() || !call.id.is_empty());
        for call in &mut calls {
            if call.id.is_empty() {
                call.id = new_id();
            }
            if call.arguments.is_empty() {
                call.arguments = "{}".into();
            }
        }
        if calls.is_empty() {
            if text.trim().is_empty() && reasoning.trim().is_empty() {
                on_event(AgentEvent::Error {
                    message: tr(lang, "empty"),
                });
            }
            session.messages.push(ChatMessage {
                id: new_id(),
                role: "assistant".into(),
                text,
                reasoning,
                tool_calls: Vec::new(),
            });
            touch(session);
            on_event(AgentEvent::Done);
            return Ok(());
        }
        let cwd = Path::new(&session.cwd);
        for call in &mut calls {
            if ask && (needs_approval(&call.name) || call.name.starts_with("mcp__")) {
                on_event(AgentEvent::ApprovalNeeded {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                });
                let allowed =
                    approve(call.id.clone(), call.name.clone(), call.arguments.clone()).await;
                if !allowed || control.cancel.load(Ordering::Relaxed) {
                    let message = tr(lang, "denied");
                    call.output = Some(message.clone());
                    call.is_error = true;
                    on_event(AgentEvent::ToolFinished {
                        id: call.id.clone(),
                        output: message,
                        is_error: true,
                    });
                    continue;
                }
            }
            if !tools.iter().any(|tool| tool.name == call.name) {
                let message = format!("tool {} is disabled", call.name);
                call.output = Some(message.clone());
                call.is_error = true;
                on_event(AgentEvent::ToolFinished {
                    id: call.id.clone(),
                    output: message,
                    is_error: true,
                });
                continue;
            }
            on_event(AgentEvent::ToolStarted {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            });
            let ToolOutput { content, is_error } =
                if let Some(output) = mcp.call(&call.name, &call.arguments).await {
                    output
                } else {
                    execute_tool(&call.name, &call.arguments, cwd).await
                };
            call.output = Some(content.clone());
            call.is_error = is_error;
            on_event(AgentEvent::ToolFinished {
                id: call.id.clone(),
                output: content,
                is_error,
            });
        }
        session.messages.push(ChatMessage {
            id: new_id(),
            role: "assistant".into(),
            text,
            reasoning,
            tool_calls: calls,
        });
        touch(session);
        exhausted = round + 1 == MAX_ROUNDS;
        if stop != Some(StopReason::ToolUse) {
            exhausted = false;
            break;
        }
    }
    if exhausted {
        session.messages.push(ChatMessage {
            id: new_id(),
            role: "assistant".into(),
            text: tr(lang, "max_rounds"),
            reasoning: String::new(),
            tool_calls: Vec::new(),
        });
        touch(session);
    }
    on_event(AgentEvent::Done);
    Ok(())
}

/// Save text that already streamed before a failure or a cancel, so the user
/// does not lose a long partial answer.
fn keep_partial(session: &mut Session, text: &mut String, reasoning: &mut String) {
    if text.trim().is_empty() && reasoning.trim().is_empty() {
        return;
    }
    session.messages.push(ChatMessage {
        id: new_id(),
        role: "assistant".into(),
        text: std::mem::take(text),
        reasoning: std::mem::take(reasoning),
        tool_calls: Vec::new(),
    });
    touch(session);
}

fn transcript(session: &Session, lang: &str, plan: bool) -> Vec<Turn> {
    let mut turns = vec![Turn::system(system_prompt(&session.cwd, lang, plan))];
    turns.extend(conversation_turns(&session.messages));
    turns
}

fn system_prompt(cwd: &str, lang: &str, plan: bool) -> String {
    let language = if lang.to_ascii_lowercase().starts_with("zh") {
        "Chinese (zh-CN)"
    } else {
        "English"
    };
    let plan_note = if plan {
        " The user turned on Plan New Idea. Do not edit files or run commands. Reply with a short numbered plan and wait for the user to leave plan mode."
    } else {
        ""
    };
    format!(
        "You are Grok Desktop, a coding agent working in {cwd}. \
Use tools to inspect and change the project. Prefer search_replace for edits of existing files. \
Do not invent file contents. Writes and shell commands may wait for the user to approve them. \
MCP tools named mcp__<server>__<tool> come from the user's stdio MCP servers. \
When a reply contains a diagram, use a ```mermaid fenced block. \
Reply in {language}.{plan_note}"
    )
}

fn title_from(text: &str, lang: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return tr(lang, "new_chat");
    }
    let mut title = trimmed.chars().take(48).collect::<String>();
    if trimmed.chars().count() > 48 {
        title.push('…');
    }
    title
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::create_session;
    use futures_util::stream;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn scripted_tool_round_edits_a_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "hello\n").unwrap();
        let mut session = create_session(
            dir.path().to_str().unwrap(),
            "deepseek",
            "deepseek-chat",
            "New chat",
        );
        let rounds = Arc::new(AtomicUsize::new(0));
        let rounds_for_stream = rounds.clone();
        let control = TurnControl {
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let mut events = Vec::new();
        drive(
            &mut session,
            "en",
            false,
            false,
            &tool_defs("en", &[]),
            &mut McpPool::empty(),
            &control,
            &mut |event| events.push(event),
            &mut |_id, _name, _args| async { true },
            |_turns| {
                let n = rounds_for_stream.fetch_add(1, Ordering::SeqCst);
                async move {
                    let events: Vec<Result<ProviderEvent, String>> = if n == 0 {
                        vec![
                            Ok(ProviderEvent::ToolStart {
                                index: 0,
                                id: "call_1".into(),
                                name: "search_replace".into(),
                            }),
                            Ok(ProviderEvent::ToolArgs {
                                index: 0,
                                fragment:
                                    "{\"path\":\"note.txt\",\"old\":\"hello\",\"new\":\"hi\"}"
                                        .into(),
                            }),
                            Ok(ProviderEvent::Stop {
                                reason: StopReason::ToolUse,
                            }),
                        ]
                    } else {
                        vec![
                            Ok(ProviderEvent::Text {
                                text: "Updated the greeting.".into(),
                            }),
                            Ok(ProviderEvent::Stop {
                                reason: StopReason::Stop,
                            }),
                        ]
                    };
                    let boxed: EventStream = Box::pin(stream::iter(events));
                    Ok(boxed)
                }
            },
        )
        .await
        .unwrap();
        let text = std::fs::read_to_string(dir.path().join("note.txt")).unwrap();
        assert_eq!(text, "hi\n");
        assert!(
            session
                .messages
                .iter()
                .any(|message| message.text.contains("Updated"))
        );
        assert!(events.iter().any(|event| matches!(event, AgentEvent::Done)));
    }

    #[tokio::test]
    async fn a_stream_error_keeps_the_partial_reply_and_is_returned() {
        let mut session = create_session("/tmp", "relay", "gpt-5", "New chat");
        session.messages.push(ChatMessage {
            id: "u1".into(),
            role: "user".into(),
            text: "hi".into(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
        });
        let control = TurnControl {
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let result = drive(
            &mut session,
            "en",
            false,
            false,
            &tool_defs("en", &[]),
            &mut McpPool::empty(),
            &control,
            &mut |_event| {},
            &mut |_id, _name, _args| async { true },
            |_turns| async {
                let events: Vec<Result<ProviderEvent, String>> = vec![
                    Ok(ProviderEvent::Text {
                        text: "half an ans".into(),
                    }),
                    Err("connection reset".into()),
                ];
                let boxed: EventStream = Box::pin(stream::iter(events));
                Ok(boxed)
            },
        )
        .await;
        assert_eq!(result.unwrap_err(), "connection reset");
        let last = session.messages.last().unwrap();
        assert_eq!(last.role, "assistant");
        assert_eq!(last.text, "half an ans");
    }
}
