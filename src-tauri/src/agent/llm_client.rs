use crate::agent::types::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResponseFormatPreference {
    #[default]
    Text,
    JsonObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChatCompletionOptions {
    pub response_format: ResponseFormatPreference,
}

impl ChatCompletionOptions {
    pub fn json_object() -> Self {
        Self {
            response_format: ResponseFormatPreference::JsonObject,
        }
    }

    pub fn wants_json_object(self) -> bool {
        matches!(self.response_format, ResponseFormatPreference::JsonObject)
    }
}

#[async_trait]
pub trait LLMClient: Send + Sync {
    async fn chat_completion(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
    ) -> Result<ChatResponse, AgentError>;

    /// M-7: the connection that actually served the most recent successful call
    /// when the client tries several (a fallback chain). Returns `None` for
    /// single-connection clients, so usage accounting falls back to the
    /// preselected route head instead of guessing.
    fn last_used_connection(&self) -> Option<UsedConnection> {
        None
    }

    async fn chat_completion_with_options(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
        options: ChatCompletionOptions,
    ) -> Result<ChatResponse, AgentError> {
        let _ = options;
        self.chat_completion(messages, tools).await
    }

    async fn chat_completion_stream(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
        event_tx: Option<Sender<AgentEvent>>,
    ) -> Result<ChatResponse, AgentError> {
        self.chat_completion_stream_with_options(
            messages,
            tools,
            event_tx,
            ChatCompletionOptions::default(),
        )
        .await
    }

    async fn chat_completion_stream_with_options(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
        event_tx: Option<Sender<AgentEvent>>,
        options: ChatCompletionOptions,
    ) -> Result<ChatResponse, AgentError> {
        let message_id = format!("msg_{}", Uuid::new_v4());
        if let Some(tx) = &event_tx {
            let _ = tx.try_send(AgentEvent::ResponseFallbackStarted {
                message_id: message_id.clone(),
                reason: "non_streaming_provider".to_string(),
            });
        }
        let response = self
            .chat_completion_with_options(messages, tools, options)
            .await?;
        if let (Some(tx), Some(content)) = (&event_tx, &response.content) {
            let _ = tx.try_send(AgentEvent::ResponseCompleted {
                message_id,
                content: content.clone(),
            });
        }
        Ok(response)
    }
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: String,
    pub usage: Option<ModelTokenUsage>,
}

/// M-7: identifies the connection that actually served a turn, so usage can be
/// attributed to the model that really answered (e.g. after a fallback
/// downgrade) instead of the preselected route head.
#[derive(Debug, Clone)]
pub struct UsedConnection {
    pub connection_id: String,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelTokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
}
