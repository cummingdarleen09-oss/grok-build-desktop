use std::pin::Pin;
use std::task::{Context, Poll};

use crate::stream::{
    AnthropicAcc, OpenAiAcc, ResponsesAcc, SseDecoder, decode_anthropic_event, decode_openai_data,
    decode_responses_data,
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

/// An unreachable host (a blocked api.x.ai, a mistyped relay) fails within this.
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// Longest silence allowed between two chunks of a streaming reply. There is no
/// total deadline, so a long answer is not cut off while it is still arriving.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

impl ProviderClient {
    pub fn new() -> Result<Self, ProviderError> {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(IDLE_TIMEOUT)
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
            responses: ResponsesAcc::default(),
            pending: std::collections::VecDeque::new(),
            utf8_tail: Vec::new(),
            saw_frame: false,
            preview: String::new(),
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
    responses: ResponsesAcc,
    pending: std::collections::VecDeque<Result<ProviderEvent, ProviderError>>,
    utf8_tail: Vec<u8>,
    saw_frame: bool,
    preview: String,
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
                    let mut rest = String::from_utf8_lossy(&this.utf8_tail).into_owned();
                    this.utf8_tail.clear();
                    rest.push_str("\n\n");
                    this.feed(&rest);
                    this.done = true;
                    if !this.saw_frame && !this.preview.trim().is_empty() {
                        this.pending.push_back(Err(ProviderError::Invalid(format!(
                            "the provider did not answer with an event stream: {}",
                            truncate(this.preview.trim(), 600)
                        ))));
                    }
                }
                Poll::Ready(Some(Err(error))) => {
                    this.done = true;
                    return Poll::Ready(Some(Err(ProviderError::Network(error))));
                }
                Poll::Ready(Some(Ok(chunk))) => {
                    this.utf8_tail.extend_from_slice(&chunk);
                    let text = take_utf8(&mut this.utf8_tail);
                    this.feed(&text);
                }
            }
        }
    }
}

impl ProviderStream {
    fn feed(&mut self, text: &str) {
        if !self.saw_frame && self.preview.len() < 2000 {
            self.preview.push_str(text);
        }
        for frame in self.decoder.push(text) {
            self.saw_frame = true;
            if frame.data.trim() == "[DONE]" {
                self.done = true;
                break;
            }
            let events = match self.format {
                ProviderFormat::OpenAi => decode_openai_data(&frame.data, &mut self.openai),
                ProviderFormat::Anthropic => {
                    decode_anthropic_event(&frame.event, &frame.data, &mut self.anthropic)
                }
                ProviderFormat::Responses => {
                    decode_responses_data(&frame.event, &frame.data, &mut self.responses)
                }
            };
            for event in events {
                self.pending.push_back(Ok(event));
            }
        }
    }
}

/// Decode the complete UTF-8 prefix of `buffer` and keep an unfinished
/// multi-byte character for the next network chunk. CJK text is three bytes
/// per character, so chunk boundaries regularly land inside one.
pub(crate) fn take_utf8(buffer: &mut Vec<u8>) -> String {
    match std::str::from_utf8(buffer) {
        Ok(text) => {
            let text = text.to_string();
            buffer.clear();
            text
        }
        Err(error) if error.error_len().is_none() => {
            let valid = error.valid_up_to();
            let rest = buffer.split_off(valid);
            let text = String::from_utf8_lossy(buffer).into_owned();
            *buffer = rest;
            text
        }
        Err(_) => {
            let text = String::from_utf8_lossy(buffer).into_owned();
            buffer.clear();
            text
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

#[cfg(test)]
mod tests {
    use super::take_utf8;

    #[test]
    fn a_character_split_across_chunks_is_kept_whole() {
        let bytes = "data: 你好\n".as_bytes();
        let split = 8;
        let mut buffer = bytes[..split].to_vec();
        let first = take_utf8(&mut buffer);
        assert_eq!(first, "data: ");
        assert_eq!(buffer.len(), 2);
        buffer.extend_from_slice(&bytes[split..]);
        let second = take_utf8(&mut buffer);
        assert_eq!(format!("{first}{second}"), "data: 你好\n");
        assert!(buffer.is_empty());
    }

    #[test]
    fn invalid_bytes_in_the_middle_do_not_stall_the_stream() {
        let mut buffer = vec![b'a', 0xff, b'b'];
        assert_eq!(take_utf8(&mut buffer), "a\u{fffd}b");
        assert!(buffer.is_empty());
    }
}
