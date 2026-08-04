//! OpenAI-compatible provider implementation
//!
//! Supports: OpenAI, Azure OpenAI, OpenRouter, DashScope (Alibaba), Ollama, llama.cpp,
//! and any other provider implementing the OpenAI Chat Completions API.

use super::{
    FunctionCall, Message, Provider, ProviderRequest, ProviderResponse, ProviderStream, ToolCall,
    Usage,
};
use futures::TryStreamExt;
use crate::error::{AgentError, Result};
use crate::tools::ToolDefinition;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// OpenAI-compatible provider
pub struct OpenAiProvider {
    client: Client,
    endpoint: String,
    api_key: Option<String>,
    model: String,
    name: String,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
}

impl OpenAiProvider {
    pub fn builder() -> OpenAiProviderBuilder {
        OpenAiProviderBuilder::default()
    }

    /// Build the full API URL
    fn api_url(&self) -> String {
        let base = self.endpoint.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else if base.ends_with("/v1") || base.ends_with("/v1/") {
            format!("{}/chat/completions", base.trim_end_matches('/'))
        } else {
            format!("{}/v1/chat/completions", base)
        }
    }
}

pub struct OpenAiProviderBuilder {
    endpoint: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    name: Option<String>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
}

impl Default for OpenAiProviderBuilder {
    fn default() -> Self {
        Self {
            endpoint: None,
            api_key: None,
            model: None,
            name: None,
            max_tokens: None,
            temperature: None,
        }
    }
}

impl OpenAiProviderBuilder {
    pub fn endpoint(mut self, endpoint: &str) -> Self {
        self.endpoint = Some(endpoint.to_string());
        self
    }

    pub fn api_key(mut self, key: &str) -> Self {
        self.api_key = Some(key.to_string());
        self
    }

    pub fn model(mut self, model: &str) -> Self {
        self.model = Some(model.to_string());
        self
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    pub fn max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn temperature(mut self, temp: f32) -> Self {
        self.temperature = Some(temp);
        self
    }

    pub fn build(self) -> Result<OpenAiProvider> {
        let endpoint = self
            .endpoint
            .ok_or_else(|| AgentError::config("endpoint is required"))?;
        let model = self
            .model
            .ok_or_else(|| AgentError::config("model is required"))?;
        let name = self.name.unwrap_or_else(|| model.clone());

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| AgentError::Internal(format!("Failed to build HTTP client: {}", e)))?;

        Ok(OpenAiProvider {
            client,
            endpoint,
            api_key: self.api_key,
            model,
            name,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        })
    }
}

// --- API Request/Response types for OpenAI Chat Completions ---

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct ApiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ApiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ApiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: ApiFunctionCall,
}

#[derive(Serialize, Deserialize, Debug)]
struct ApiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct ApiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: ApiFunction,
}

#[derive(Serialize)]
struct ApiFunction {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: serde_json::Value,
}

#[derive(Deserialize, Debug)]
struct ApiResponse {
    #[serde(default)]
    choices: Vec<ApiChoice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ApiChoice {
    message: ApiResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ApiResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ApiToolCall>>,
    /// Some models put reasoning/thinking here
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

#[derive(Deserialize, Debug)]
struct ApiError {
    error: ApiErrorDetail,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct ApiErrorDetail {
    message: String,
    #[serde(default)]
    #[serde(rename = "type")]
    error_type: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

// --- Streaming chunk shapes ---

#[derive(Deserialize, Debug)]
struct ApiStreamChunk {
    #[serde(default)]
    choices: Vec<ApiStreamChoice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ApiStreamChoice {
    #[serde(default)]
    delta: ApiStreamDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct ApiStreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ApiStreamToolCall>>,
}

#[derive(Deserialize, Debug)]
struct ApiStreamToolCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ApiStreamFunction>,
}

#[derive(Deserialize, Debug)]
struct ApiStreamFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// Parse one SSE `data:` payload into a `StreamChunk`.
pub fn parse_stream_chunk(data: &str) -> crate::error::Result<crate::provider::StreamChunk> {
    let chunk: ApiStreamChunk = serde_json::from_str(data)
        .map_err(|e| crate::error::AgentError::Provider(format!("bad stream chunk: {}", e)))?;
    let mut out = crate::provider::StreamChunk {
        model: chunk.model,
        usage: chunk.usage.map(|u| crate::provider::Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        }),
        ..Default::default()
    };
    if let Some(choice) = chunk.choices.into_iter().next() {
        out.delta_content = choice.delta.content;
        out.delta_reasoning = choice.delta.reasoning_content;
        out.finish_reason = choice.finish_reason;
        for tc in choice.delta.tool_calls.unwrap_or_default() {
            out.tool_call_deltas.push(crate::provider::ToolCallDelta {
                index: tc.index,
                id: tc.id,
                name_delta: tc.function.as_ref().and_then(|f| f.name.clone()),
                arguments_delta: tc.function.as_ref().and_then(|f| f.arguments.clone()),
            });
        }
    }
    Ok(out)
}

// --- Conversion helpers ---

fn message_to_api(msg: &Message) -> ApiMessage {
    ApiMessage {
        role: msg.role.to_string(),
        content: msg.content.clone(),
        tool_calls: msg.tool_calls.as_ref().map(|calls| {
            calls
                .iter()
                .map(|c| ApiToolCall {
                    id: c.id.clone(),
                    call_type: c.call_type.clone(),
                    function: ApiFunctionCall {
                        name: c.function.name.clone(),
                        arguments: c.function.arguments.clone(),
                    },
                })
                .collect()
        }),
        tool_call_id: msg.tool_call_id.clone(),
        name: msg.name.clone(),
    }
}

fn tool_def_to_api(tool: &ToolDefinition) -> ApiTool {
    ApiTool {
        tool_type: "function".to_string(),
        function: ApiFunction {
            name: tool.name.clone(),
            description: Some(tool.description.clone()),
            parameters: tool.parameters.clone(),
        },
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    async fn chat_completion(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        let url = self.api_url();
        debug!("OpenAI API call to: {}", url);

        let api_messages: Vec<ApiMessage> = request.messages.iter().map(message_to_api).collect();

        let api_tools: Option<Vec<ApiTool>> = if request.tools.is_empty() {
            None
        } else {
            Some(request.tools.iter().map(tool_def_to_api).collect())
        };

        let api_request = ApiRequest {
            model: request.model.clone(),
            messages: api_messages,
            tools: api_tools,
            max_tokens: request.max_tokens.or(self.max_tokens),
            temperature: request.temperature.or(self.temperature),
            stop: request.stop,
            stream: false,
            stream_options: None,
        };

        let mut req_builder = self.client.post(&url).header("Content-Type", "application/json");

        if let Some(ref key) = self.api_key {
            req_builder = req_builder.bearer_auth(key);
        }

        let response = req_builder
            .json(&api_request)
            .send()
            .await
            .map_err(|e| AgentError::Provider(format!("HTTP request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            // Try to parse structured error
            if let Ok(api_err) = serde_json::from_str::<ApiError>(&error_body) {
                return Err(AgentError::Provider(format!(
                    "API error ({}): {}",
                    status, api_err.error.message
                )));
            }
            return Err(AgentError::Provider(format!(
                "API error ({}): {}",
                status,
                error_body.chars().take(500).collect::<String>()
            )));
        }

        let api_response: ApiResponse = response
            .json()
            .await
            .map_err(|e| AgentError::Provider(format!("Failed to parse response: {}", e)))?;

        let choice = api_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| AgentError::Provider("No choices in response".to_string()))?;

        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.id,
                call_type: tc.call_type,
                function: FunctionCall {
                    name: tc.function.name,
                    arguments: tc.function.arguments,
                },
            })
            .collect();

        let usage = api_response.usage.map(|u| Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        });

        Ok(ProviderResponse {
            content: choice.message.content,
            tool_calls,
            usage,
            model: api_response.model.unwrap_or(request.model),
            reasoning: choice.message.reasoning_content,
            finish_reason: choice.finish_reason,
        })
    }

    async fn chat_completion_stream(&self, request: ProviderRequest) -> Result<ProviderStream> {
        let url = self.api_url();
        debug!("OpenAI streaming call to: {}", url);

        let api_messages: Vec<ApiMessage> = request.messages.iter().map(message_to_api).collect();
        let api_tools: Option<Vec<ApiTool>> = if request.tools.is_empty() {
            None
        } else {
            Some(request.tools.iter().map(tool_def_to_api).collect())
        };
        let api_request = ApiRequest {
            model: request.model.clone(),
            messages: api_messages,
            tools: api_tools,
            max_tokens: request.max_tokens.or(self.max_tokens),
            temperature: request.temperature.or(self.temperature),
            stop: request.stop,
            stream: true,
            stream_options: Some(serde_json::json!({"include_usage": true})),
        };

        let mut req_builder = self.client.post(&url).header("Content-Type", "application/json");
        if let Some(ref key) = self.api_key {
            req_builder = req_builder.bearer_auth(key);
        }
        let response = req_builder
            .json(&api_request)
            .send()
            .await
            .map_err(|e| AgentError::Provider(format!("HTTP request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            if let Ok(api_err) = serde_json::from_str::<ApiError>(&error_body) {
                return Err(AgentError::Provider(format!(
                    "API error ({}): {}",
                    status, api_err.error.message
                )));
            }
            return Err(AgentError::Provider(format!(
                "API error ({}): {}",
                status,
                error_body.chars().take(500).collect::<String>()
            )));
        }

        let byte_stream = response.bytes_stream();
        let reader = SseLineReader {
            stream: Box::pin(byte_stream),
            buf: Vec::new(),
            eof: false,
        };
        let chunk_stream = futures::stream::unfold(reader, |mut reader| async move {
            loop {
                let line = match reader.next_line().await {
                    Some(line) => line,
                    None => return None, // EOF
                };
                let line = line.trim().to_string();
                if line.is_empty() || line.starts_with(':') {
                    continue; // blank separator / SSE comment (keepalive)
                }
                let Some(data) = line.strip_prefix("data:") else {
                    continue; // event:/id:/retry: lines are not used
                };
                let data = data.trim();
                if data == "[DONE]" {
                    return None;
                }
                return Some((parse_stream_chunk(data), reader));
            }
        });
        Ok(Box::pin(chunk_stream))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn analyze_image(&self, prompt: &str, image_url: &str) -> Result<String> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": prompt},
                    {"type": "image_url", "image_url": {"url": image_url}}
                ]
            }],
            "max_tokens": self.max_tokens.unwrap_or(1024),
        });
        let mut request = self.client.post(self.api_url()).json(&body);
        if let Some(ref key) = self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await
            .map_err(|e| AgentError::provider(format!("vision request failed: {}", e)))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AgentError::provider(format!("vision API {}: {}", status, &text[..text.len().min(300)])));
        }
        let payload: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AgentError::provider(format!("parse vision response: {}", e)))?;
        payload
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| AgentError::provider("vision response missing content"))
    }
}


/// Incremental SSE line reader over a byte stream.
struct SseLineReader {
    stream: std::pin::Pin<
        Box<dyn futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Send>,
    >,
    buf: Vec<u8>,
    eof: bool,
}

impl SseLineReader {
    /// Next complete line (without trailing \n), or None at EOF.
    async fn next_line(&mut self) -> Option<String> {
        loop {
            if let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=pos).collect();
                let line = line
                    .strip_suffix(b"\n".as_slice())
                    .map(|l| l.to_vec())
                    .unwrap_or(line);
                let line = line
                    .strip_suffix(b"\r".as_slice())
                    .map(|l| l.to_vec())
                    .unwrap_or(line);
                return Some(String::from_utf8_lossy(&line).to_string());
            }
            if self.eof {
                if self.buf.is_empty() {
                    return None;
                }
                let rest: Vec<u8> = self.buf.drain(..).collect();
                return Some(String::from_utf8_lossy(&rest).to_string());
            }
            match self.stream.try_next().await {
                Ok(Some(bytes)) => self.buf.extend_from_slice(&bytes),
                Ok(None) => self.eof = true,
                Err(_) => self.eof = true, // treat transport errors as EOF
            }
        }
    }
}

#[cfg(test)]
mod streaming_tests {
    use super::*;
    use crate::provider::assemble_tool_calls;

    #[test]
    fn test_parse_content_chunk() {
        let chunk = parse_stream_chunk(
            r#"{"id":"x","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#,
        )
        .unwrap();
        assert_eq!(chunk.delta_content.as_deref(), Some("Hel"));
        assert!(chunk.finish_reason.is_none());
    }

    #[test]
    fn test_parse_tool_call_deltas() {
        let c1 = parse_stream_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"get_time","arguments":""}}]}}]}"#,
        )
        .unwrap();
        let c2 = parse_stream_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"tz\":"}}]}}]}"#,
        )
        .unwrap();
        let c3 = parse_stream_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"UTC\"}"}}]}}],"model":"m"}"#,
        )
        .unwrap();
        let mut deltas = vec![];
        deltas.extend(c1.tool_call_deltas);
        deltas.extend(c2.tool_call_deltas);
        deltas.extend(c3.tool_call_deltas);
        let calls = assemble_tool_calls(&deltas);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].function.name, "get_time");
        assert_eq!(calls[0].function.arguments, r#"{"tz":"UTC"}"#);
    }

    #[test]
    fn test_parse_final_usage_chunk() {
        let chunk = parse_stream_chunk(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":7,"total_tokens":12}}"#,
        )
        .unwrap();
        assert_eq!(chunk.finish_reason.as_deref(), Some("stop"));
        assert_eq!(chunk.usage.as_ref().unwrap().total_tokens, 12);
    }

    #[tokio::test]
    async fn test_sse_line_reader() {
        let data = b"data: one\n\ndata: two\n: keepalive\ndata: [DONE]\n";
        let stream = futures::stream::iter(vec![
            Ok(bytes::Bytes::from(&data[..11])),
            Ok(bytes::Bytes::from(&data[11..])),
        ]);
        let mut reader = SseLineReader {
            stream: Box::pin(stream),
            buf: Vec::new(),
            eof: false,
        };
        let mut lines = Vec::new();
        while let Some(line) = reader.next_line().await {
            lines.push(line);
        }
        assert_eq!(
            lines,
            vec!["data: one", "", "data: two", ": keepalive", "data: [DONE]"]
        );
    }
}
