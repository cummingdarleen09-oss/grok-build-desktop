use std::collections::BTreeMap;

use serde_json::Value;

use crate::types::{ProviderEvent, Role, StopReason, ToolCall, Turn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Incremental SSE frame decoder. A blank line dispatches one event.
#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: String,
    event_name: String,
    data: String,
}

impl SseDecoder {
    pub fn push(&mut self, chunk: &str) -> Vec<SseEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();
        while let Some(position) = self.buffer.find('\n') {
            let mut line = self.buffer.drain(..=position).collect::<String>();
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            if line.is_empty() {
                if !self.data.is_empty() {
                    if self.data.ends_with('\n') {
                        self.data.pop();
                    }
                    events.push(SseEvent {
                        event: std::mem::take(&mut self.event_name),
                        data: std::mem::take(&mut self.data),
                    });
                } else {
                    self.event_name.clear();
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(rest.trim_start());
            } else if let Some(rest) = line.strip_prefix("event:") {
                self.event_name = rest.trim().to_string();
            }
        }
        events
    }
}

#[derive(Debug, Default)]
pub struct OpenAiAcc {
    started: BTreeMap<u32, bool>,
}

pub fn decode_openai_data(data: &str, state: &mut OpenAiAcc) -> Vec<ProviderEvent> {
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    let mut events = Vec::new();
    if let Some(usage) = value.get("usage") {
        events.push(ProviderEvent::Usage {
            input_tokens: json_u64(usage.get("prompt_tokens")),
            output_tokens: json_u64(usage.get("completion_tokens")),
        });
    }
    let Some(choice) = value
        .get("choices")
        .and_then(|choices| choices.as_array())
        .and_then(|choices| choices.first())
    else {
        return events;
    };
    if let Some(delta) = choice.get("delta") {
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                events.push(ProviderEvent::Text {
                    text: text.to_string(),
                });
            }
        }
        if let Some(text) = delta
            .get("reasoning_content")
            .and_then(Value::as_str)
            .or_else(|| delta.get("reasoning").and_then(Value::as_str))
        {
            if !text.is_empty() {
                events.push(ProviderEvent::Reasoning {
                    text: text.to_string(),
                });
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
                let id = call.get("id").and_then(Value::as_str).unwrap_or("");
                let function = call.get("function");
                let name = function
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !state.started.contains_key(&index) && (!id.is_empty() || !name.is_empty()) {
                    state.started.insert(index, true);
                    events.push(ProviderEvent::ToolStart {
                        index,
                        id: id.to_string(),
                        name: name.to_string(),
                    });
                }
                if let Some(fragment) = function
                    .and_then(|function| function.get("arguments"))
                    .and_then(Value::as_str)
                {
                    if !fragment.is_empty() {
                        events.push(ProviderEvent::ToolArgs {
                            index,
                            fragment: fragment.to_string(),
                        });
                    }
                }
            }
        }
    }
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        events.push(ProviderEvent::Stop {
            reason: map_openai_stop(reason),
        });
    }
    events
}

fn map_openai_stop(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::Stop,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "length" => StopReason::Length,
        _ => StopReason::Other,
    }
}

#[derive(Debug, Default)]
pub struct AnthropicAcc {
    blocks: BTreeMap<u32, String>,
}

pub fn decode_anthropic_event(
    event: &str,
    data: &str,
    state: &mut AnthropicAcc,
) -> Vec<ProviderEvent> {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    let kind = if event.is_empty() {
        value.get("type").and_then(Value::as_str).unwrap_or("")
    } else {
        event
    };
    let mut events = Vec::new();
    match kind {
        "message_start" => {
            if let Some(usage) = value.pointer("/message/usage") {
                events.push(ProviderEvent::Usage {
                    input_tokens: json_u64(usage.get("input_tokens")),
                    output_tokens: json_u64(usage.get("output_tokens")),
                });
            }
        }
        "content_block_start" => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
            let block = value.get("content_block");
            let block_type = block
                .and_then(|block| block.get("type"))
                .and_then(Value::as_str);
            if block_type == Some("tool_use") {
                let id = block
                    .and_then(|block| block.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let name = block
                    .and_then(|block| block.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                state.blocks.insert(index, "tool_use".into());
                events.push(ProviderEvent::ToolStart {
                    index,
                    id: id.to_string(),
                    name: name.to_string(),
                });
            } else if block_type == Some("thinking") {
                state.blocks.insert(index, "thinking".into());
            } else {
                state.blocks.insert(index, "text".into());
            }
        }
        "content_block_delta" => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
            let delta = value.get("delta");
            let delta_type = delta
                .and_then(|delta| delta.get("type"))
                .and_then(Value::as_str);
            match delta_type {
                Some("text_delta") => {
                    if let Some(text) = delta
                        .and_then(|delta| delta.get("text"))
                        .and_then(Value::as_str)
                    {
                        if !text.is_empty() {
                            events.push(ProviderEvent::Text {
                                text: text.to_string(),
                            });
                        }
                    }
                }
                Some("thinking_delta") => {
                    if let Some(text) = delta
                        .and_then(|delta| delta.get("thinking"))
                        .and_then(Value::as_str)
                    {
                        if !text.is_empty() {
                            events.push(ProviderEvent::Reasoning {
                                text: text.to_string(),
                            });
                        }
                    }
                }
                Some("input_json_delta") => {
                    if let Some(fragment) = delta
                        .and_then(|delta| delta.get("partial_json"))
                        .and_then(Value::as_str)
                    {
                        if !fragment.is_empty() {
                            events.push(ProviderEvent::ToolArgs {
                                index,
                                fragment: fragment.to_string(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        "message_delta" => {
            if let Some(usage) = value.get("usage") {
                let output = json_u64(usage.get("output_tokens"));
                if output > 0 {
                    events.push(ProviderEvent::Usage {
                        input_tokens: json_u64(usage.get("input_tokens")),
                        output_tokens: output,
                    });
                }
            }
            if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                events.push(ProviderEvent::Stop {
                    reason: map_anthropic_stop(reason),
                });
            }
        }
        _ => {}
    }
    events
}

fn map_anthropic_stop(reason: &str) -> StopReason {
    match reason {
        "end_turn" | "stop_sequence" => StopReason::Stop,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::Length,
        _ => StopReason::Other,
    }
}

fn json_u64(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

/// Accumulates streaming events into one assistant turn.
#[derive(Debug, Default)]
pub struct AssistantBuild {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: BTreeMap<u32, ToolCall>,
    pub stop: Option<StopReason>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl AssistantBuild {
    pub fn apply(&mut self, event: ProviderEvent) {
        match event {
            ProviderEvent::Text { text } => self.text.push_str(&text),
            ProviderEvent::Reasoning { text } => self.reasoning.push_str(&text),
            ProviderEvent::ToolStart { index, id, name } => {
                self.tool_calls.entry(index).or_insert(ToolCall {
                    id,
                    name,
                    arguments: String::new(),
                });
            }
            ProviderEvent::ToolArgs { index, fragment } => {
                let call = self.tool_calls.entry(index).or_insert(ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                });
                call.arguments.push_str(&fragment);
            }
            ProviderEvent::Usage {
                input_tokens,
                output_tokens,
            } => {
                if input_tokens > 0 {
                    self.input_tokens = input_tokens;
                }
                if output_tokens > 0 {
                    self.output_tokens = output_tokens;
                }
            }
            ProviderEvent::Stop { reason } => self.stop = Some(reason),
        }
    }

    pub fn into_turn(self) -> Turn {
        Turn {
            role: Role::Assistant,
            text: self.text,
            reasoning: if self.reasoning.is_empty() {
                None
            } else {
                Some(self.reasoning)
            },
            tool_calls: self.tool_calls.into_values().collect(),
            tool_call_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_sse_parses_text_tools_and_usage() {
        let raw = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"He\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"llo\",\"reasoning_content\":\"hmm\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":7,\"total_tokens\":19}}\n\n",
            "data: [DONE]\n\n",
        );
        let mut decoder = SseDecoder::default();
        let mut openai = OpenAiAcc::default();
        let mut build = AssistantBuild::default();
        for frame in decoder.push(raw) {
            for event in decode_openai_data(&frame.data, &mut openai) {
                build.apply(event);
            }
        }
        assert_eq!(build.text, "Hello");
        assert_eq!(build.reasoning, "hmm");
        assert_eq!(build.stop, Some(StopReason::ToolUse));
        assert_eq!(build.input_tokens, 12);
        assert_eq!(build.output_tokens, 7);
        let calls: Vec<_> = build.tool_calls.values().collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments, "{\"path\":\"a.rs\"}");
    }

    #[test]
    fn anthropic_sse_parses_text_thinking_and_tool_use() {
        let raw = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"plan\"}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"read_file\",\"input\":{}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":20}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let mut decoder = SseDecoder::default();
        let mut state = AnthropicAcc::default();
        let mut build = AssistantBuild::default();
        for frame in decoder.push(raw) {
            for event in decode_anthropic_event(&frame.event, &frame.data, &mut state) {
                build.apply(event);
            }
        }
        assert_eq!(build.text, "Hi");
        assert_eq!(build.reasoning, "plan");
        assert_eq!(build.stop, Some(StopReason::ToolUse));
        assert_eq!(build.input_tokens, 10);
        assert_eq!(build.output_tokens, 20);
        let call = build.tool_calls.get(&2).unwrap();
        assert_eq!(call.id, "toolu_1");
        assert_eq!(call.name, "read_file");
        assert_eq!(call.arguments, "{\"path\":\"a.rs\"}");
    }

    #[test]
    fn sse_decoder_handles_split_chunks() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push("data: {\"a\":").is_empty());
        let frames = decoder.push("1}\n\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, "{\"a\":1}");
    }
}
