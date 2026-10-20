use std::time::Duration;

use futures_util::StreamExt;
use grok_providers::{
    CompletionRequest, ProviderClient, ProviderEvent, StopReason, Turn, builtin_presets,
};
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn preset(id: &str) -> grok_providers::ProviderConfig {
    let mut provider = builtin_presets()
        .iter()
        .copied()
        .find(|preset| preset.id == id)
        .unwrap()
        .to_config(true);
    provider.max_tokens = 32;
    provider
}

#[tokio::test]
async fn openai_compatible_mock_checks_translation_and_stream() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"pong\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\"type\":\"function\",\"function\":{\"name\":\"list_dir\",\"arguments\":\"{\\\"path\\\":\\\".\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer sk-deepseek"))
        .and(body_partial_json(json!({
            "model": "deepseek-chat",
            "stream": true,
            "max_tokens": 32,
            "messages": [{"role": "user", "content": "ping"}]
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut provider = preset("deepseek");
    provider.base_url = server.uri();
    let client = ProviderClient::new().unwrap();
    let turns = vec![Turn::user("ping")];
    let mut stream = client
        .stream(CompletionRequest {
            provider: &provider,
            api_key: "sk-deepseek",
            turns: &turns,
            tools: &[],
            effort: None,
        })
        .await
        .unwrap();
    let mut text = String::new();
    let mut tool_name = String::new();
    let mut stop = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            ProviderEvent::Text { text: delta } => text.push_str(&delta),
            ProviderEvent::ToolStart { name, .. } => tool_name = name,
            ProviderEvent::Stop { reason } => stop = Some(reason),
            _ => {}
        }
        if tokio::time::Instant::now() > deadline {
            panic!("stream did not finish");
        }
    }
    assert_eq!(text, "pong");
    assert_eq!(tool_name, "list_dir");
    assert_eq!(stop, Some(StopReason::ToolUse));
}

#[tokio::test]
async fn anthropic_mock_uses_x_api_key_and_messages_path() {
    let server = MockServer::start().await;
    let sse = concat!(
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_partial_json(json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 32,
            "system": "be brief",
            "messages": [{"role": "user", "content": "hi"}]
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut provider = preset("anthropic");
    provider.base_url = format!("{}/v1", server.uri());
    let client = ProviderClient::new().unwrap();
    let turns = vec![Turn::system("be brief"), Turn::user("hi")];
    let text = client
        .complete_text(&provider, "sk-ant-test", &turns)
        .await
        .unwrap();
    assert_eq!(text, "ok");
}

#[tokio::test]
async fn responses_mock_streams_text_and_function_calls() {
    let server = MockServer::start().await;
    let sse = concat!(
        "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"pong\"}\n\n",
        "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"name\":\"list_dir\",\"call_id\":\"call_9\",\"arguments\":\"\"}}\n\n",
        "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"delta\":\"{\\\"path\\\":\\\".\\\"}\"}\n\n",
        "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer sk-xai"))
        .and(body_partial_json(json!({
            "model": "grok-4.7",
            "stream": true,
            "max_output_tokens": 32,
            "reasoning": {"effort": "xhigh"}
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut provider = preset("xai");
    provider.base_url = format!("{}/v1", server.uri());
    provider.model = "grok-4.7".into();
    let client = ProviderClient::new().unwrap();
    let turns = vec![Turn::user("ping")];
    let mut stream = client
        .stream(CompletionRequest {
            provider: &provider,
            api_key: "sk-xai",
            turns: &turns,
            tools: &[],
            effort: Some(grok_providers::ReasoningEffort::Xhigh),
        })
        .await
        .unwrap();
    let mut text = String::new();
    let mut tool_name = String::new();
    let mut stop = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            ProviderEvent::Text { text: delta } => text.push_str(&delta),
            ProviderEvent::ToolStart { name, .. } => tool_name = name,
            ProviderEvent::Stop { reason } => stop = Some(reason),
            _ => {}
        }
        if tokio::time::Instant::now() > deadline {
            panic!("stream did not finish");
        }
    }
    assert_eq!(text, "pong");
    assert_eq!(tool_name, "list_dir");
    assert_eq!(stop, Some(StopReason::ToolUse));
}

#[tokio::test]
async fn http_error_is_redacted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad key sk-secret-value"))
        .mount(&server)
        .await;
    let mut provider = preset("kimi");
    provider.base_url = format!("{}/v1", server.uri());
    let client = ProviderClient::new().unwrap();
    let turns = vec![Turn::user("hi")];
    let error = match client
        .stream(CompletionRequest {
            provider: &provider,
            api_key: "sk-secret-value",
            turns: &turns,
            tools: &[],
            effort: None,
        })
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("expected HTTP error"),
    };
    let rendered = error.to_string();
    assert!(rendered.contains("401"), "{rendered}");
    assert!(!rendered.contains("sk-secret-value"), "{rendered}");
    assert!(rendered.contains("•••"), "{rendered}");
}

#[tokio::test]
async fn a_json_body_instead_of_an_event_stream_is_reported() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"error":{"message":"model not found: deepseek-chat"}}"#),
        )
        .mount(&server)
        .await;
    let mut provider = preset("deepseek");
    provider.base_url = server.uri();
    let client = ProviderClient::new().unwrap();
    let turns = vec![Turn::user("ping")];
    let mut stream = client
        .stream(CompletionRequest {
            provider: &provider,
            api_key: "sk-deepseek",
            turns: &turns,
            tools: &[],
            effort: None,
        })
        .await
        .unwrap();
    let mut error = None;
    while let Some(event) = stream.next().await {
        if let Err(failure) = event {
            error = Some(failure.to_string());
        }
    }
    let error = error.expect("a non-SSE body must surface as an error");
    assert!(error.contains("model not found"), "{error}");
}

#[tokio::test]
async fn an_unreachable_host_fails_with_a_readable_cause() {
    let mut provider = preset("deepseek");
    provider.base_url = "http://127.0.0.1:9".into();
    let client = ProviderClient::new().unwrap();
    let turns = vec![Turn::user("ping")];
    let error = match client
        .stream(CompletionRequest {
            provider: &provider,
            api_key: "sk-deepseek",
            turns: &turns,
            tools: &[],
            effort: None,
        })
        .await
    {
        Ok(_) => panic!("port 9 should refuse the connection"),
        Err(error) => error.detailed(),
    };
    assert!(error.to_ascii_lowercase().contains("connect"), "{error}");
}
