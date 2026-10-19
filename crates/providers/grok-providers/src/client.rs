use std::pin::Pin;
use std::task::{Context, Poll};

use crate::stream::{
    AnthropicAcc, OpenAiAcc, SseDecoder, decode_anthropic_event, decode_openai_data,
};
use crate::translate::{endpoint_url, request_body};
use crate::types::{
    ProviderConfig, ProviderError, ProviderEvent, ProviderFormat, ToolDef, Turn, redact_secret,
};
use futures_util::{Stream, StreamExt};

#[derive(Debug, Clone)]
pub struct CompletionRequest<'a> {
    pub provider: &'a ProviderConfig,
    pub api_key: &'a str,
    pub turns: &'a [Turn],
    pub tools: &'a [ToolDef],
}

pub struct ProviderClient {
    http: reqwest::Client,
}

impl ProviderClient {
    pub fn new() -> Result<Self, ProviderError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;
        Ok(Self { http })
    }

    pub async fn stream(
        &self,
        request: CompletionRequest<'_>,
    ) -> Result<ProviderStream, ProviderError> {
        if request.api_key.trim().is_empty() {
            return Err(ProviderError::MissingKey(request.provider.id.clone()));
        }
        let url = endpoint_url(
            &request.provider.base_url,
            request.provider.format.path_suffix(),
        )?;
        let body = request_body(request.provider, request.turns, request.tools)?;
        let mut builder = self
            .http
            .post(url)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(&body);
        builder = match request.provider.auth {
            crate::types::AuthStyle::Bearer => builder.bearer_auth(request.api_key),
            crate::types::AuthStyle::XApiKey => builder.header("x-api-key", request.api_key),
        };
        for (name, value) in &request.provider.extra_headers {
            builder = builder.header(name, value);
        }
        let response = builder.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            let body = truncate(&redact_secret(&text, request.api_key), 2000);
            return Err(ProviderError::Http { status, body });
        }
        Ok(ProviderStream {
            bytes: Box::pin(response.bytes_stream()),
            decoder: SseDecoder::default(),
            format: request.provider.format,
            openai: OpenAiAcc::default(),
            anthropic: AnthropicAcc::default(),
            pending: std::collections::VecDeque::new(),
            done: false,
        })
    }

    /// Non-streaming convenience used by the settings "test connection" button.
    pub async fn complete_text(
        &self,
        provider: &ProviderConfig,
        api_key: &str,
        turns: &[Turn],
    ) -> Result<String, ProviderError> {
        let mut stream = self
            .stream(CompletionRequest {
                provider,
                api_key,
                turns,
                tools: &[],
            })
            .await?;
        let mut text = String::new();
        while let Some(event) = stream.next().await {
            match event? {
                ProviderEvent::Text { text: delta } => text.push_str(&delta),
                ProviderEvent::Stop { .. } => break,
                _ => {}
            }
        }
        Ok(text)
    }
}

impl Default for ProviderClient {
    fn default() -> Self {
        Self::new().expect("reqwest client")
    }
}

type ByteStream = Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>;

pub struct ProviderStream {
    bytes: ByteStream,
    decoder: SseDecoder,
    format: ProviderFormat,
    openai: OpenAiAcc,
    anthropic: AnthropicAcc,
    pending: std::collections::VecDeque<Result<ProviderEvent, ProviderError>>,
    done: bool,
}

impl Stream for ProviderStream {
    type Item = Result<ProviderEvent, ProviderError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(event) = this.pending.pop_front() {
                return Poll::Ready(Some(event));
            }
            if this.done {
                return Poll::Ready(None);
            }
            match Pin::new(&mut this.bytes).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    this.done = true;
                    return Poll::Ready(None);
                }
                Poll::Ready(Some(Err(error))) => {
                    this.done = true;
                    return Poll::Ready(Some(Err(ProviderError::Network(error))));
                }
                Poll::Ready(Some(Ok(chunk))) => {
                    let text = String::from_utf8_lossy(&chunk);
                    for frame in this.decoder.push(&text) {
                        if frame.data.trim() == "[DONE]" {
                            this.done = true;
                            break;
                        }
                        let events = match this.format {
                            ProviderFormat::OpenAi => {
                                decode_openai_data(&frame.data, &mut this.openai)
                            }
                            ProviderFormat::Anthropic => decode_anthropic_event(
                                &frame.event,
                                &frame.data,
                                &mut this.anthropic,
                            ),
                        };
                        for event in events {
                            this.pending.push_back(Ok(event));
                        }
                    }
                }
            }
        }
    }
}

/// Exposed so callers can show a shortened provider error without the API key.
fn truncate(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let mut out = text.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}
