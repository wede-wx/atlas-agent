//! P3-3: a fallback-aware LLM client.
//!
//! [`FallbackLLMClient`] wraps an ordered chain of per-connection clients (built
//! from the model route decision's `fallback_chain`). When an earlier connection
//! fails at call time it tries the next one, and — crucially for the card's
//! "no silent swap" line — emits a visible `Thinking` event recording the
//! downgrade. A user cancellation (`AgentError::Cancelled`) is never retried.

use async_trait::async_trait;
use tokio::sync::mpsc::Sender;

use crate::agent::llm_client::{ChatCompletionOptions, ChatResponse, LLMClient, UsedConnection};
use crate::agent::types::{AgentError, AgentEvent, Message};
use crate::agent::ToolSchema;

/// One link in the fallback chain: a built client plus a human-readable label.
pub struct FallbackClientEntry {
    /// `provider/model`, used in the downgrade event so the user can tell which
    /// model actually served the turn.
    pub label: String,
    pub connection_id: String,
    /// M-7: structured provider/model so usage is attributed to the connection
    /// that actually answered, without re-parsing `label`.
    pub provider: String,
    pub model: String,
    pub client: Box<dyn LLMClient>,
}

impl FallbackClientEntry {
    pub fn new(
        label: impl Into<String>,
        connection_id: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        client: Box<dyn LLMClient>,
    ) -> Self {
        Self {
            label: label.into(),
            connection_id: connection_id.into(),
            provider: provider.into(),
            model: model.into(),
            client,
        }
    }
}

/// An LLM client that tries an ordered chain of connections, falling back to the
/// next on a (non-cancellation) error and recording each downgrade.
pub struct FallbackLLMClient {
    entries: Vec<FallbackClientEntry>,
    /// M-7: records the connection that served the latest successful call so
    /// the agent loop can attribute usage to the model that actually answered.
    last_used: std::sync::Mutex<Option<UsedConnection>>,
}

impl FallbackLLMClient {
    pub fn new(entries: Vec<FallbackClientEntry>) -> Self {
        Self {
            entries,
            last_used: std::sync::Mutex::new(None),
        }
    }

    fn record_used(&self, entry: &FallbackClientEntry) {
        if let Ok(mut slot) = self.last_used.lock() {
            *slot = Some(UsedConnection {
                connection_id: entry.connection_id.clone(),
                provider: entry.provider.clone(),
                model: entry.model.clone(),
            });
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn no_connection_error() -> AgentError {
        AgentError::Llm("没有可用的模型连接。".to_string())
    }

    fn emit_fallback_note(
        event_tx: &Option<Sender<AgentEvent>>,
        failed: &FallbackClientEntry,
        next: &FallbackClientEntry,
        error: &AgentError,
    ) {
        if let Some(tx) = event_tx {
            let _ = tx.try_send(AgentEvent::Thinking {
                content: format!(
                    "模型 {} 调用失败（{}）。已自动降级到备用模型 {} 继续。",
                    failed.label,
                    brief_error(error),
                    next.label
                ),
            });
        }
    }
}

#[async_trait]
impl LLMClient for FallbackLLMClient {
    async fn chat_completion(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
    ) -> Result<ChatResponse, AgentError> {
        self.chat_completion_with_options(messages, tools, ChatCompletionOptions::default())
            .await
    }

    async fn chat_completion_with_options(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
        options: ChatCompletionOptions,
    ) -> Result<ChatResponse, AgentError> {
        let mut last_error = Self::no_connection_error();
        for entry in &self.entries {
            match entry
                .client
                .chat_completion_with_options(messages.clone(), tools.clone(), options)
                .await
            {
                Ok(response) => {
                    self.record_used(entry);
                    return Ok(response);
                }
                // Never fall back on a user cancellation.
                Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }

    async fn chat_completion_stream_with_options(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolSchema>>,
        event_tx: Option<Sender<AgentEvent>>,
        options: ChatCompletionOptions,
    ) -> Result<ChatResponse, AgentError> {
        let last_index = self.entries.len().saturating_sub(1);
        let mut last_error = Self::no_connection_error();
        for (index, entry) in self.entries.iter().enumerate() {
            match entry
                .client
                .chat_completion_stream_with_options(
                    messages.clone(),
                    tools.clone(),
                    event_tx.clone(),
                    options,
                )
                .await
            {
                Ok(response) => {
                    self.record_used(entry);
                    return Ok(response);
                }
                Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                Err(error) => {
                    if index < last_index {
                        // Record the downgrade so it is never silent.
                        Self::emit_fallback_note(
                            &event_tx,
                            entry,
                            &self.entries[index + 1],
                            &error,
                        );
                    }
                    last_error = error;
                }
            }
        }
        Err(last_error)
    }

    fn last_used_connection(&self) -> Option<UsedConnection> {
        self.last_used.lock().ok().and_then(|slot| slot.clone())
    }
}

fn brief_error(error: &AgentError) -> String {
    let text = error.to_string();
    let trimmed = text.trim();
    let max = 160;
    if trimmed.chars().count() > max {
        let head: String = trimmed.chars().take(max).collect();
        format!("{head}…")
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A scripted client: succeeds with a tagged response or fails with a given
    /// error, counting how many times it was called.
    enum Script {
        Succeed(String),
        FailLlm(String),
        FailCancelled,
    }

    struct ScriptedClient {
        script: Script,
        calls: Arc<AtomicUsize>,
    }

    impl ScriptedClient {
        fn ok(tag: &str, calls: Arc<AtomicUsize>) -> Self {
            Self {
                script: Script::Succeed(tag.to_string()),
                calls,
            }
        }

        fn fail_llm(message: &str, calls: Arc<AtomicUsize>) -> Self {
            Self {
                script: Script::FailLlm(message.to_string()),
                calls,
            }
        }

        fn fail_cancelled(calls: Arc<AtomicUsize>) -> Self {
            Self {
                script: Script::FailCancelled,
                calls,
            }
        }
    }

    #[async_trait]
    impl LLMClient for ScriptedClient {
        async fn chat_completion(
            &self,
            _messages: Vec<Message>,
            _tools: Option<Vec<ToolSchema>>,
        ) -> Result<ChatResponse, AgentError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.script {
                Script::Succeed(tag) => Ok(ChatResponse {
                    content: Some(tag.clone()),
                    tool_calls: vec![],
                    finish_reason: "stop".to_string(),
                    usage: None,
                }),
                Script::FailLlm(message) => Err(AgentError::Llm(message.clone())),
                Script::FailCancelled => Err(AgentError::Cancelled),
            }
        }
    }

    fn entry(tag: &str, client: ScriptedClient) -> FallbackClientEntry {
        FallbackClientEntry::new(
            tag,
            tag,
            format!("prov-{tag}"),
            format!("model-{tag}"),
            Box::new(client),
        )
    }

    #[tokio::test]
    async fn primary_failure_falls_back_to_next_and_records_event() {
        let p_calls = Arc::new(AtomicUsize::new(0));
        let s_calls = Arc::new(AtomicUsize::new(0));
        let client = FallbackLLMClient::new(vec![
            entry(
                "primary",
                ScriptedClient::fail_llm("503 upstream", p_calls.clone()),
            ),
            entry(
                "secondary",
                ScriptedClient::ok("secondary", s_calls.clone()),
            ),
        ]);
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        let response = client
            .chat_completion_stream_with_options(
                vec![],
                None,
                Some(tx),
                ChatCompletionOptions::default(),
            )
            .await
            .expect("fallback should succeed on secondary");

        assert_eq!(response.content.as_deref(), Some("secondary"));
        assert_eq!(p_calls.load(Ordering::SeqCst), 1);
        assert_eq!(s_calls.load(Ordering::SeqCst), 1);

        // A downgrade note must be emitted (not silent).
        let mut saw_downgrade = false;
        while let Ok(event) = rx.try_recv() {
            if let AgentEvent::Thinking { content } = event {
                if content.contains("降级") && content.contains("secondary") {
                    saw_downgrade = true;
                }
            }
        }
        assert!(saw_downgrade, "fallback must emit a visible downgrade note");

        // M-7: usage must be attributable to the connection that actually
        // served the turn (secondary), not the route head (primary).
        let used = client
            .last_used_connection()
            .expect("a used connection must be recorded after success");
        assert_eq!(used.connection_id, "secondary");
        assert_eq!(used.provider, "prov-secondary");
        assert_eq!(used.model, "model-secondary");
    }

    #[tokio::test]
    async fn cancellation_is_not_retried() {
        let p_calls = Arc::new(AtomicUsize::new(0));
        let s_calls = Arc::new(AtomicUsize::new(0));
        let client = FallbackLLMClient::new(vec![
            entry("primary", ScriptedClient::fail_cancelled(p_calls.clone())),
            entry(
                "secondary",
                ScriptedClient::ok("secondary", s_calls.clone()),
            ),
        ]);

        let result = client.chat_completion(vec![], None).await;

        assert!(matches!(result, Err(AgentError::Cancelled)));
        assert_eq!(p_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            s_calls.load(Ordering::SeqCst),
            0,
            "cancel must not fall back"
        );
    }

    #[tokio::test]
    async fn all_connections_failing_returns_last_error() {
        let client = FallbackLLMClient::new(vec![
            entry(
                "primary",
                ScriptedClient::fail_llm("first", Arc::new(AtomicUsize::new(0))),
            ),
            entry(
                "secondary",
                ScriptedClient::fail_llm("last", Arc::new(AtomicUsize::new(0))),
            ),
        ]);

        let result = client.chat_completion(vec![], None).await;
        match result {
            Err(AgentError::Llm(message)) => assert_eq!(message, "last"),
            other => panic!("expected last Llm error, got {other:?}"),
        }
        // M-7: every connection failed, so there is nothing to attribute.
        assert!(
            client.last_used_connection().is_none(),
            "no success must mean no usage attribution"
        );
    }

    #[tokio::test]
    async fn single_healthy_connection_serves_without_fallback() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = FallbackLLMClient::new(vec![entry(
            "only",
            ScriptedClient::ok("only", calls.clone()),
        )]);

        let response = client.chat_completion(vec![], None).await.unwrap();
        assert_eq!(response.content.as_deref(), Some("only"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // M-7: even a single healthy connection records what served the turn.
        let used = client
            .last_used_connection()
            .expect("used connection recorded");
        assert_eq!(used.model, "model-only");
    }
}
