use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};

pub struct StopRunTool {
    cancel_tokens: Option<Arc<Mutex<HashMap<String, CancellationToken>>>>,
    current_session_id: Option<String>,
}

impl StopRunTool {
    pub fn unavailable() -> Self {
        Self {
            cancel_tokens: None,
            current_session_id: None,
        }
    }

    pub fn new(
        cancel_tokens: Arc<Mutex<HashMap<String, CancellationToken>>>,
        current_session_id: Option<String>,
    ) -> Self {
        Self {
            cancel_tokens: Some(cancel_tokens),
            current_session_id,
        }
    }
}

#[async_trait]
impl Tool for StopRunTool {
    fn name(&self) -> &str {
        "stop_run"
    }

    fn description(&self) -> &str {
        "Cancel the current Atlas Agent run."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description:
                "Cancel the current Atlas Agent run. Use only when the user clearly asks to stop."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "Optional session id. Defaults to the current session."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Short reason"
                    }
                }
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "停止任务".to_string(),
            description_zh: "取消当前 Agent 任务运行。".to_string(),
            capability_labels_zh: vec!["运行控制".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::System],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let Some(tokens) = &self.cancel_tokens else {
            return Err(AgentError::Tool(
                "停止任务工具只在真实 Agent 运行中可用。".to_string(),
            ));
        };
        let session_id = args
            .get("sessionId")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| self.current_session_id.clone())
            .ok_or_else(|| AgentError::Tool("缺少要停止的会话。".to_string()))?;
        let key = session_id.clone();
        let token = {
            let mut active = tokens.lock().await;
            active.remove(&key)
        };
        if let Some(token) = token {
            token.cancel();
            Ok(ToolResult::success(
                "已发送停止任务请求。",
                serde_json::json!({ "sessionId": session_id, "cancelled": true }),
            ))
        } else {
            Ok(ToolResult::warning(
                "没有找到正在运行的任务。",
                serde_json::json!({ "sessionId": session_id, "cancelled": false }),
                vec!["告诉用户当前会话没有正在运行的任务。".to_string()],
            ))
        }
    }
}
