use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::StreamExt;
use grok_providers::{
    ProviderClient, ProviderConfig, ProviderEvent, ReasoningEffort, StopReason, ToolDef, Turn,
};
use serde::Serialize;

use crate::mcp::McpPool;
use crate::sessions::{
    COMPACTION_ROLE, ChatMessage, Session, StoredTool, conversation_turns, touch,
};
use crate::settings::ModelDefault;
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

/// The effort a turn sends and the context window it works within.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnOptions {
    pub effort: Option<ReasoningEffort>,
    pub context_window: u64,
    /// Auto-compaction starts when the context reaches this share of the window.
    pub compact_at_percent: u8,
    /// Sent as `x-compaction-at` until the chat is first compacted.
    pub compaction_at: Option<u64>,
}

impl TurnOptions {
    /// The chat's choice when the model offers it, then the saved default for
    /// the model, then the model's own default.
    pub fn resolve(provider: &ProviderConfig, session: &Session, saved: &ModelDefault) -> Self {
        let chat = ModelDefault {
            reasoning_effort: session.reasoning_effort,
            context_window: session.context_window,
        };
        Self::choose(provider, &chat, saved)
    }

    /// [`Self::resolve`] for a chat that may not exist yet.
    pub fn choose(provider: &ProviderConfig, chat: &ModelDefault, saved: &ModelDefault) -> Self {
        let options = provider.model_options(&provider.model);
        let offered_effort =
            |value: &ReasoningEffort| options.efforts.iter().any(|option| option.value == *value);
        let effort = options.effort(
            chat.reasoning_effort
                .filter(offered_effort)
                .or(saved.reasoning_effort.filter(offered_effort)),
        );
        let offered_window = |window: &u64| options.context_windows.contains(window);
        let context_window = options.context_window(
            chat.context_window
                .filter(offered_window)
                .or(saved.context_window.filter(offered_window)),
        );
        let compact_at_percent = options.auto_compact_threshold_percent;
        Self {
            effort,
            context_window,
            compact_at_percent,
            compaction_at: options
                .compaction_at_tokens
                .and_then(|at| at.resolve(context_window, compact_at_percent)),
        }
    }
}

/// What one request carries besides the transcript.
#[derive(Debug, Clone, Copy)]
struct RequestShape {
    tools: bool,
    compaction_at: Option<u64>,
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
    options: TurnOptions,
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
        reasoning_signature: String::new(),
    };
    if session.messages.is_empty() {
        session.title = title_from(user_text, lang);
    }
    session.provider_id = provider.id.clone();
    session.model = provider.model.clone();
    session.messages.push(user);
    touch(session);

    let open_stream = |turns: &[Turn], shape: RequestShape| {
        let turns = turns.to_vec();
        let tools = if shape.tools {
            tools.clone()
        } else {
            Vec::new()
        };
        let provider = match shape.compaction_at {
            Some(tokens) => {
                let mut provider = provider.clone();
                provider
                    .extra_headers
                    .insert("x-compaction-at".into(), tokens.to_string());
                std::borrow::Cow::Owned(provider)
            }
            None => std::borrow::Cow::Borrowed(provider),
        };
        async move {
            if api_key.trim().is_empty() {
                return Err(tr(lang, "missing_key"));
            }
            let stream = client
                .stream(grok_providers::CompletionRequest {
                    provider: &provider,
                    api_key,
                    turns: &turns,
                    tools: &tools,
                    effort: options.effort,
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
        options,
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
    options: TurnOptions,
    on_event: &mut impl FnMut(AgentEvent),
    approve: &mut F,
    mut open_stream: Open,
) -> Result<(), String>
where
    F: FnMut(String, String, String) -> Fut,
    Fut: Future<Output = bool>,
    Open: FnMut(&[Turn], RequestShape) -> OpenFut,
    OpenFut: Future<Output = Result<EventStream, String>>,
{
    let mut exhausted = false;
    // Tokens the provider reported for the last request: its prompt plus its reply.
    let mut reported: u64 = 0;
    for round in 0..MAX_ROUNDS {
        if control.cancel.load(Ordering::Relaxed) {
            on_event(AgentEvent::Error {
                message: tr(lang, "cancelled"),
            });
            on_event(AgentEvent::Done);
            return Ok(());
        }
        let used = estimate_context(&transcript(session, lang, plan), tools).max(reported);
        let mut compacted = false;
        if xai_token_estimation::exceeds_threshold(
            used,
            options.context_window,
            options.compact_at_percent,
        ) {
            compacted = compact(
                session,
                lang,
                plan,
                used,
                options,
                control,
                on_event,
                &mut open_stream,
            )
            .await
            .is_ok();
            if compacted {
                reported = 0;
            }
        }
        let shape = |session: &Session| RequestShape {
            tools: true,
            compaction_at: options.compaction_at.filter(|_| !is_compacted(session)),
        };
        let turns = transcript(session, lang, plan);
        let mut stream = match open_stream(&turns, shape(session)).await {
            Ok(stream) => stream,
            // The CLI compacts and retries once when the provider says the prompt is too long.
            Err(error) if !compacted && xai_grok_compaction::is_context_length_error(&error) => {
                let used = estimate_context(&turns, tools).max(options.context_window);
                if compact(
                    session,
                    lang,
                    plan,
                    used,
                    options,
                    control,
                    on_event,
                    &mut open_stream,
                )
                .await
                .is_err()
                {
                    return Err(error);
                }
                reported = 0;
                let turns = transcript(session, lang, plan);
                open_stream(&turns, shape(session)).await?
            }
            Err(error) => return Err(error),
        };
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut signature = String::new();
        let mut usage = (0u64, 0u64);
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
                ProviderEvent::ReasoningSignature { signature: part } => {
                    signature.push_str(&part);
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
                // Anthropic reports input at the start and output at the end.
                ProviderEvent::Usage {
                    input_tokens,
                    output_tokens,
                } => {
                    if input_tokens > 0 {
                        usage.0 = input_tokens;
                    }
                    if output_tokens > 0 {
                        usage.1 = output_tokens;
                    }
                }
                ProviderEvent::Stop { reason } => stop = Some(reason),
            }
        }
        if let Some(error) = failure {
            keep_partial(session, &mut text, &mut reasoning);
            return Err(error);
        }
        if usage.0.saturating_add(usage.1) > 0 {
            reported = usage.0.saturating_add(usage.1);
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
                reasoning_signature: signature,
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
            reasoning_signature: signature,
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
            reasoning_signature: String::new(),
        });
        touch(session);
    }
    on_event(AgentEvent::Done);
    Ok(())
}

/// Context size the CLI's way (`xai-token-estimation`: four bytes a token),
/// tool schemas included because every request carries them.
fn estimate_context(turns: &[Turn], tools: &[ToolDef]) -> u64 {
    let mut total = 0u64;
    for turn in turns {
        total += xai_token_estimation::estimate_tokens(&turn.text);
        if let Some(reasoning) = &turn.reasoning {
            total += xai_token_estimation::estimate_tokens(reasoning);
        }
        for call in &turn.tool_calls {
            total += xai_token_estimation::estimate_tokens(&call.name)
                + xai_token_estimation::estimate_tokens(&call.arguments);
        }
    }
    for tool in tools {
        total += xai_token_estimation::estimate_tokens(&tool.description)
            + xai_token_estimation::estimate_tokens(&tool.parameters.to_string());
    }
    total
}

fn is_compacted(session: &Session) -> bool {
    session
        .messages
        .iter()
        .any(|message| message.role == COMPACTION_ROLE)
}

/// grok-build's full-replace compaction (`xai-grok-compaction`
/// `code_compaction`): the model summarizes the conversation with the CLI's
/// summary prompt, and a compaction message carrying the latest request and
/// that summary replaces everything before it in later requests. The full
/// history stays in the chat for the user to read.
#[allow(clippy::too_many_arguments)]
async fn compact<Open, OpenFut>(
    session: &mut Session,
    lang: &str,
    plan: bool,
    used: u64,
    options: TurnOptions,
    control: &TurnControl,
    on_event: &mut impl FnMut(AgentEvent),
    open_stream: &mut Open,
) -> Result<(), String>
where
    Open: FnMut(&[Turn], RequestShape) -> OpenFut,
    OpenFut: Future<Output = Result<EventStream, String>>,
{
    on_event(AgentEvent::Status {
        text: tr(lang, "compacting")
            .replace("{used}", &used.to_string())
            .replace("{window}", &options.context_window.to_string())
            .replace("{percent}", &options.compact_at_percent.to_string()),
    });
    let mut turns = transcript(session, lang, plan);
    // A summary request has to fit too: keep the newest turns within the
    // window and start the kept part at a user turn so no tool result is orphaned.
    let budget = options.context_window.saturating_mul(9) / 10;
    while turns.len() > 2 && estimate_context(&turns, &[]) > budget {
        turns.remove(1);
        while turns.len() > 2 && turns[1].role != grok_providers::Role::User {
            turns.remove(1);
        }
    }
    turns.push(Turn::user(xai_grok_compaction::build_summary_prompt(None)));
    let result = async {
        let mut stream = open_stream(
            &turns,
            RequestShape {
                tools: false,
                compaction_at: None,
            },
        )
        .await?;
        let mut summary = String::new();
        while let Some(event) = stream.next().await {
            if control.cancel.load(Ordering::Relaxed) {
                return Err(tr(lang, "cancelled"));
            }
            match event? {
                ProviderEvent::Text { text } => summary.push_str(&text),
                ProviderEvent::Stop { .. } => break,
                _ => {}
            }
        }
        if summary.trim().is_empty() {
            return Err(tr(lang, "empty"));
        }
        Ok(summary)
    }
    .await;
    let summary = match result {
        Ok(summary) => summary,
        Err(error) => {
            on_event(AgentEvent::Status {
                text: tr(lang, "compact_failed").replace("{error}", &error),
            });
            return Err(error);
        }
    };
    let last_query = session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message.text.clone());
    let mut text = String::new();
    if let Some(query) = last_query {
        text.push_str(&xai_grok_compaction::wrap_user_query(query));
        text.push_str("\n\n");
    }
    text.push_str(&xai_grok_compaction::format_compact_summary_content(
        &summary,
    ));
    session.messages.push(ChatMessage {
        id: new_id(),
        role: COMPACTION_ROLE.into(),
        text,
        reasoning: String::new(),
        reasoning_signature: String::new(),
        tool_calls: Vec::new(),
    });
    touch(session);
    on_event(AgentEvent::Status {
        text: tr(lang, "compacted"),
    });
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
        reasoning_signature: String::new(),
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
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    fn wide() -> TurnOptions {
        TurnOptions {
            effort: None,
            context_window: 500_000,
            compact_at_percent: 85,
            compaction_at: None,
        }
    }

    fn reply(text: &str) -> EventStream {
        Box::pin(stream::iter(vec![
            Ok(ProviderEvent::Text { text: text.into() }),
            Ok(ProviderEvent::Stop {
                reason: StopReason::Stop,
            }),
        ]))
    }

    fn long_chat(cwd: &str) -> Session {
        let mut session = create_session(cwd, "xai", "grok-4.7", "New chat");
        for index in 0..6 {
            session.messages.push(ChatMessage {
                id: format!("u{index}"),
                role: "user".into(),
                text: format!("question {index} {}", "x".repeat(2_000)),
                reasoning: String::new(),
                reasoning_signature: String::new(),
                tool_calls: Vec::new(),
            });
            session.messages.push(ChatMessage {
                id: format!("a{index}"),
                role: "assistant".into(),
                text: format!("answer {index} {}", "y".repeat(2_000)),
                reasoning: String::new(),
                reasoning_signature: String::new(),
                tool_calls: Vec::new(),
            });
        }
        session.messages.push(ChatMessage {
            id: "now".into(),
            role: "user".into(),
            text: "the latest request".into(),
            reasoning: String::new(),
            reasoning_signature: String::new(),
            tool_calls: Vec::new(),
        });
        session
    }

    type Seen = Arc<Mutex<Vec<(Vec<Turn>, bool, Option<u64>)>>>;

    async fn run_scripted(
        session: &mut Session,
        options: TurnOptions,
        replies: Vec<Result<&'static str, &'static str>>,
    ) -> (Result<(), String>, Seen, Vec<AgentEvent>) {
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let replies = Arc::new(Mutex::new(replies.into_iter()));
        let control = TurnControl {
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let mut events = Vec::new();
        let seen_for_stream = seen.clone();
        let result = drive(
            session,
            "en",
            false,
            false,
            &[],
            &mut McpPool::empty(),
            &control,
            options,
            &mut |event| events.push(event),
            &mut |_id, _name, _args| async { true },
            |turns, shape| {
                seen_for_stream.lock().unwrap().push((
                    turns.to_vec(),
                    shape.tools,
                    shape.compaction_at,
                ));
                let next = replies.lock().unwrap().next().expect("a scripted reply");
                async move {
                    match next {
                        Ok(text) => Ok(reply(text)),
                        Err(error) => Err(error.to_string()),
                    }
                }
            },
        )
        .await;
        (result, seen, events)
    }

    #[tokio::test]
    async fn a_chat_past_the_threshold_is_compacted_before_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = long_chat(dir.path().to_str().unwrap());
        let options = TurnOptions {
            context_window: 4_000,
            compact_at_percent: 80,
            compaction_at: Some(3_200),
            ..wide()
        };
        let (result, seen, events) = run_scripted(
            &mut session,
            options,
            vec![Ok("SUMMARY of six questions"), Ok("done")],
        )
        .await;
        result.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        let (summary_turns, summary_tools, summary_header) = &seen[0];
        assert!(!summary_tools);
        assert_eq!(*summary_header, None);
        assert!(summary_turns.last().unwrap().text.contains("summary"));
        let (turns, tools, header) = &seen[1];
        assert!(tools);
        assert_eq!(*header, None, "x-compaction-at stops once compacted");
        assert_eq!(turns.len(), 2, "system prompt plus the compaction message");
        assert!(
            turns[1]
                .text
                .contains("<user_query>\nthe latest request\n</user_query>")
        );
        assert!(turns[1].text.contains("SUMMARY of six questions"));
        assert!(
            turns[1]
                .text
                .contains("continued from a previous conversation")
        );
        let roles: Vec<&str> = session.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles[roles.len() - 2..], ["compaction", "assistant"]);
        assert_eq!(
            session.messages.len(),
            15,
            "nothing is deleted from the chat"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Status { text } if text.contains("4000 tokens")
        )));
    }

    #[tokio::test]
    async fn the_same_chat_in_a_bigger_window_is_sent_whole() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = long_chat(dir.path().to_str().unwrap());
        let options = TurnOptions {
            compaction_at: Some(400_000),
            ..wide()
        };
        let (result, seen, _) = run_scripted(&mut session, options, vec![Ok("done")]).await;
        result.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0.len(), 14);
        assert_eq!(seen[0].2, Some(400_000));
        assert!(session.messages.iter().all(|m| m.role != "compaction"));
    }

    #[tokio::test]
    async fn a_context_length_error_compacts_once_and_retries() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = long_chat(dir.path().to_str().unwrap());
        let (result, seen, _) = run_scripted(
            &mut session,
            wide(),
            vec![
                Err("HTTP 400: This model's maximum context length is 8192 tokens"),
                Ok("short summary"),
                Ok("done"),
            ],
        )
        .await;
        result.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert!(!seen[1].1);
        assert_eq!(seen[2].0.len(), 2);
        assert_eq!(session.messages.last().unwrap().text, "done");
    }

    #[tokio::test]
    async fn a_second_compaction_starts_from_the_first_summary() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = long_chat(dir.path().to_str().unwrap());
        let small = TurnOptions {
            context_window: 4_000,
            compact_at_percent: 80,
            ..wide()
        };
        run_scripted(&mut session, small, vec![Ok("first summary"), Ok("done")])
            .await
            .0
            .unwrap();
        let turns = conversation_turns(&session.messages);
        assert_eq!(turns.len(), 2);
        assert!(turns[0].text.contains("first summary"));
        assert_eq!(turns[1].text, "done");
    }

    #[test]
    fn options_prefer_the_chat_then_the_saved_default_then_the_model() {
        let mut store = grok_providers::default_store();
        let xai = store
            .providers
            .iter_mut()
            .find(|provider| provider.id == "xai")
            .unwrap();
        xai.model = "grok-4.6".into();
        let provider = xai.clone();
        let mut session = create_session("/tmp", "xai", "grok-4.6", "t");
        let none = ModelDefault::default();

        let model = TurnOptions::resolve(&provider, &session, &none);
        assert_eq!(model.effort, Some(ReasoningEffort::High));
        assert_eq!(model.context_window, 500_000);
        assert_eq!(model.compact_at_percent, 80);
        assert_eq!(model.compaction_at, Some(400_000));

        let saved = ModelDefault {
            reasoning_effort: Some(ReasoningEffort::Low),
            context_window: Some(123),
        };
        let from_saved = TurnOptions::resolve(&provider, &session, &saved);
        assert_eq!(from_saved.effort, Some(ReasoningEffort::Low));
        assert_eq!(from_saved.context_window, 500_000, "123 is not offered");

        session.reasoning_effort = Some(ReasoningEffort::Xhigh);
        let from_chat = TurnOptions::resolve(&provider, &session, &saved);
        assert_eq!(from_chat.effort, Some(ReasoningEffort::Xhigh));

        session.reasoning_effort = Some(ReasoningEffort::Max);
        let unoffered = TurnOptions::resolve(&provider, &session, &saved);
        assert_eq!(unoffered.effort, Some(ReasoningEffort::Low));

        let mut fast = provider.clone();
        fast.model = "grok-4.7-fast".into();
        let fast_options = TurnOptions::resolve(&fast, &session, &saved);
        assert_eq!(fast_options.effort, None);
    }

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
            wide(),
            &mut |event| events.push(event),
            &mut |_id, _name, _args| async { true },
            |_turns, _shape| {
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
            reasoning_signature: String::new(),
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
            wide(),
            &mut |_event| {},
            &mut |_id, _name, _args| async { true },
            |_turns, _shape| async {
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
