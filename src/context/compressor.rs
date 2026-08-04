//! Context compression — port of hermes' agent/conversation_compression.py
//!
//! When the conversation exceeds the token budget, the middle turns are
//! summarized by a secondary model call and replaced with a compact
//! "[CONTEXT SUMMARY]" message, keeping the system prompt, the first user
//! message, and the most recent turns intact.

use crate::provider::{Message, Provider, Role};

const SUMMARY_PROMPT: &str = "Summarize the conversation segment below into a compact briefing \
that preserves: decisions made, facts learned, file paths touched, pending work, and any \
constraints the user stated. Be dense and factual; bullet points are fine. Do not address the \
user — this summary is injected back into the model's context.\n\nCONVERSATION SEGMENT:\n";

pub struct ContextCompressor {
    /// Maximum context tokens before compression kicks in.
    pub max_context_tokens: usize,
    /// Target ratio of the budget the compressed result should fit under.
    pub target_ratio: f32,
    /// Number of recent messages always kept verbatim.
    pub keep_recent: usize,
}

impl Default for ContextCompressor {
    fn default() -> Self {
        Self::new(100_000)
    }
}

impl ContextCompressor {
    pub fn new(max_context_tokens: usize) -> Self {
        Self {
            max_context_tokens,
            target_ratio: 0.5,
            keep_recent: 12,
        }
    }

    /// Estimate token count (rough approximation: 4 chars per token).
    pub fn estimate_tokens(messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|m| {
                let content_len = m.content.as_ref().map(|c| c.len()).unwrap_or(0);
                let tool_len = m
                    .tool_calls
                    .as_ref()
                    .map(|calls| serde_json::to_string(calls).map(|s| s.len()).unwrap_or(0))
                    .unwrap_or(0);
                (content_len + tool_len) / 4
            })
            .sum()
    }

    /// Check if compression is needed.
    pub fn needs_compression(&self, messages: &[Message]) -> bool {
        Self::estimate_tokens(messages) > self.max_context_tokens
    }

    /// Compress using a provider call. Returns None when compression fails or
    /// is not applicable (keeps the original messages).
    pub async fn compress_with_provider(
        &self,
        messages: Vec<Message>,
        provider: &dyn Provider,
    ) -> Option<Vec<Message>> {
        if messages.len() <= self.keep_recent + 2 {
            return None;
        }
        // Keep: system prompt + first user message + recent tail.
        let system = messages.iter().find(|m| m.role == Role::System).cloned();
        let first_user = messages.iter().find(|m| m.role == Role::User).cloned();
        let tail_start = messages.len() - self.keep_recent;
        let middle: Vec<&Message> = messages
            .iter()
            .enumerate()
            .filter(|(i, m)| m.role != Role::System && *i > 0 && *i < tail_start)
            .map(|(_, m)| m)
            .collect();
        if middle.is_empty() {
            return None;
        }

        let mut segment = String::new();
        for message in &middle {
            let content = message.content.as_deref().unwrap_or("");
            if content.is_empty() {
                continue;
            }
            // Bound each message to keep the summary prompt manageable.
            let bounded: String = content.chars().take(2000).collect();
            segment.push_str(&format!("[{}] {}\n", message.role, bounded));
        }
        if segment.is_empty() {
            return None;
        }

        let summary_request = crate::provider::ProviderRequest {
            messages: vec![Message {
                role: Role::User,
                content: Some(format!("{}{}", SUMMARY_PROMPT, segment)),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            tools: vec![],
            model: provider.model().to_string(),
            max_tokens: Some(1024),
            temperature: Some(0.2),
            stream: false,
            stop: None,
        };

        let summary = match provider.chat_completion(summary_request).await {
            Ok(response) => response.content.unwrap_or_default(),
            Err(e) => {
                tracing::warn!("compression failed: {}", e);
                return None;
            }
        };
        if summary.trim().is_empty() {
            return None;
        }

        let mut compressed = Vec::new();
        if let Some(system) = system {
            compressed.push(system);
        }
        if let Some(first_user) = first_user {
            compressed.push(first_user);
        }
        compressed.push(Message {
            role: Role::User,
            content: Some(format!(
                "[CONTEXT SUMMARY — earlier turns compressed]\n{}",
                summary.trim()
            )),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
        compressed.push(Message {
            role: Role::Assistant,
            content: Some("Understood — continuing from the summary.".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
        compressed.extend(messages.into_iter().skip(tail_start));
        Some(compressed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimation_and_threshold() {
        let compressor = ContextCompressor::new(10);
        let messages = vec![Message {
            role: Role::User,
            content: Some("x".repeat(100)), // ~25 tokens
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];
        assert!(compressor.needs_compression(&messages));
        let big = ContextCompressor::new(100_000);
        assert!(!big.needs_compression(&messages));
    }
}
