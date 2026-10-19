use serde_json::{Value, json};

use crate::types::{
    ProviderConfig, ProviderError, ProviderFormat, Role, TokenField, ToolDef, Turn,
};

/// Join `base_url` and an API suffix without duplicating the suffix or dropping a query.
pub fn endpoint_url(base_url: &str, suffix: &str) -> Result<String, ProviderError> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(ProviderError::Invalid("base URL is empty".into()));
    }
    let (origin, query) = match trimmed.split_once('?') {
        Some((origin, query)) => (origin.trim_end_matches('/'), Some(query)),
        None => (trimmed, None),
    };
    let suffix = suffix.trim_start_matches('/');
    let url = if origin.ends_with(&format!("/{suffix}")) || origin.ends_with(suffix) {
        origin.to_string()
    } else {
        format!("{origin}/{suffix}")
    };
    Ok(match query {
        Some(query) if !query.is_empty() => format!("{url}?{query}"),
        _ => url,
    })
}

pub fn openai_request_body(
    provider: &ProviderConfig,
    turns: &[Turn],
    tools: &[ToolDef],
) -> Result<Value, ProviderError> {
    let mut body = json!({
        "model": provider.model,
        "messages": openai_messages(turns)?,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    match provider.token_field {
        TokenField::MaxTokens => {
            body["max_tokens"] = json!(provider.max_tokens);
        }
        TokenField::MaxCompletionTokens => {
            body["max_completion_tokens"] = json!(provider.max_tokens);
        }
    }
    if let Some(temperature) = provider.temperature {
        body["temperature"] = json!(temperature);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.iter().map(openai_tool).collect());
    }
    Ok(body)
}

pub fn anthropic_request_body(
    provider: &ProviderConfig,
    turns: &[Turn],
    tools: &[ToolDef],
) -> Result<Value, ProviderError> {
    let (system, messages) = anthropic_messages(turns)?;
    let mut body = json!({
        "model": provider.model,
        "max_tokens": provider.max_tokens,
        "messages": messages,
        "stream": true,
    });
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if let Some(temperature) = provider.temperature {
        body["temperature"] = json!(temperature);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.iter().map(anthropic_tool).collect());
    }
    Ok(body)
}

pub fn request_body(
    provider: &ProviderConfig,
    turns: &[Turn],
    tools: &[ToolDef],
) -> Result<Value, ProviderError> {
    match provider.format {
        ProviderFormat::OpenAi => openai_request_body(provider, turns, tools),
        ProviderFormat::Anthropic => anthropic_request_body(provider, turns, tools),
    }
}

fn openai_messages(turns: &[Turn]) -> Result<Vec<Value>, ProviderError> {
    let mut messages = Vec::with_capacity(turns.len());
    for turn in turns {
        let role = match turn.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let mut message = json!({ "role": role });
        if turn.role == Role::Tool {
            let id = turn.tool_call_id.clone().ok_or_else(|| {
                ProviderError::Invalid("tool turn is missing tool_call_id".into())
            })?;
            message["tool_call_id"] = json!(id);
            message["content"] = json!(turn.text);
        } else if turn.role == Role::Assistant {
            message["content"] = if turn.text.is_empty() && !turn.tool_calls.is_empty() {
                Value::Null
            } else {
                json!(turn.text)
            };
            if let Some(reasoning) = turn.reasoning.as_ref().filter(|text| !text.is_empty()) {
                message["reasoning_content"] = json!(reasoning);
            }
            if !turn.tool_calls.is_empty() {
                message["tool_calls"] = Value::Array(
                    turn.tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": call.arguments,
                                }
                            })
                        })
                        .collect(),
                );
            }
        } else {
            message["content"] = json!(turn.text);
        }
        messages.push(message);
    }
    Ok(messages)
}

fn anthropic_messages(turns: &[Turn]) -> Result<(String, Vec<Value>), ProviderError> {
    let mut system = String::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut index = 0;
    while index < turns.len() {
        let turn = &turns[index];
        match turn.role {
            Role::System => {
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(&turn.text);
                index += 1;
            }
            Role::User => {
                messages.push(json!({
                    "role": "user",
                    "content": turn.text,
                }));
                index += 1;
            }
            Role::Assistant => {
                let mut blocks = Vec::new();
                if !turn.text.is_empty() {
                    blocks.push(json!({"type": "text", "text": turn.text}));
                }
                for call in &turn.tool_calls {
                    let input = parse_tool_arguments(&call.arguments)?;
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": input,
                    }));
                }
                if blocks.is_empty() {
                    blocks.push(json!({"type": "text", "text": ""}));
                }
                messages.push(json!({"role": "assistant", "content": blocks}));
                index += 1;
            }
            Role::Tool => {
                let mut blocks = Vec::new();
                while index < turns.len() && turns[index].role == Role::Tool {
                    let tool_turn = &turns[index];
                    let id = tool_turn.tool_call_id.clone().ok_or_else(|| {
                        ProviderError::Invalid("tool turn is missing tool_call_id".into())
                    })?;
                    blocks.push(json!({
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": tool_turn.text,
                    }));
                    index += 1;
                }
                messages.push(json!({"role": "user", "content": blocks}));
            }
        }
    }
    Ok((system, messages))
}

fn parse_tool_arguments(arguments: &str) -> Result<Value, ProviderError> {
    if arguments.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(arguments)
        .map_err(|error| ProviderError::Invalid(format!("tool arguments are not JSON: {error}")))
}

fn openai_tool(tool: &ToolDef) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

fn anthropic_tool(tool: &ToolDef) -> Value {
    json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.parameters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolCall;

    fn provider(format: ProviderFormat) -> ProviderConfig {
        ProviderConfig {
            id: "test".into(),
            name: "Test".into(),
            format,
            base_url: "https://example.test/v1".into(),
            model: "demo-model".into(),
            env_key: "TEST_KEY".into(),
            env_key_fallbacks: Vec::new(),
            context_window: 1000,
            extra_headers: BTreeMap::new(),
            env_http_headers: BTreeMap::new(),
            auth: AuthStyle::Bearer,
            enabled: true,
            temperature: None,
            max_tokens: 128,
            token_field: if format == ProviderFormat::OpenAi {
                TokenField::MaxCompletionTokens
            } else {
                TokenField::MaxTokens
            },
            builtin: false,
            model_hints: Vec::new(),
        }
    }

    use crate::types::{AuthStyle, TokenField};
    use std::collections::BTreeMap;

    fn sample_turns() -> Vec<Turn> {
        vec![
            Turn::system("be brief"),
            Turn::user("read a.rs"),
            Turn {
                role: Role::Assistant,
                text: String::new(),
                reasoning: Some("look at the file".into()),
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "read_file".into(),
                    arguments: "{\"path\":\"a.rs\"}".into(),
                }],
                tool_call_id: None,
            },
            Turn::tool("call_1", "fn main() {}"),
        ]
    }

    #[test]
    fn openai_body_translates_tools_reasoning_and_token_field() {
        let body = openai_request_body(
            &provider(ProviderFormat::OpenAi),
            &sample_turns(),
            &[ToolDef {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            }],
        )
        .unwrap();
        assert_eq!(body["model"], "demo-model");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_completion_tokens"], 128);
        assert!(body.get("temperature").is_none());
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][2]["reasoning_content"], "look at the file");
        assert_eq!(body["messages"][2]["content"], Value::Null);
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["name"],
            "read_file"
        );
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "call_1");
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
    }

    #[test]
    fn anthropic_body_lifts_system_and_groups_tool_results() {
        let mut provider = provider(ProviderFormat::Anthropic);
        provider.temperature = Some(0.2);
        let body = anthropic_request_body(
            &provider,
            &sample_turns(),
            &[ToolDef {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: json!({"type": "object"}),
            }],
        )
        .unwrap();
        assert_eq!(body["system"], "be brief");
        assert_eq!(body["max_tokens"], 128);
        let temperature = body["temperature"].as_f64().unwrap();
        assert!((temperature - 0.2).abs() < 1e-6, "{temperature}");
        assert_eq!(body["messages"].as_array().unwrap().len(), 3);
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][1]["content"][0]["input"]["path"], "a.rs");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "call_1");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert!(
            body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .all(|message| message["role"] != "system")
        );
    }

    #[test]
    fn endpoint_joins_suffix_and_keeps_query() {
        assert_eq!(
            endpoint_url("https://api.deepseek.com", "chat/completions").unwrap(),
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(
            endpoint_url("https://api.moonshot.cn/v1/", "chat/completions").unwrap(),
            "https://api.moonshot.cn/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url("https://api.anthropic.com/v1", "messages").unwrap(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            endpoint_url(
                "https://gateway.example/v1/chat/completions?api-version=x",
                "chat/completions"
            )
            .unwrap(),
            "https://gateway.example/v1/chat/completions?api-version=x"
        );
    }
}
