use async_trait::async_trait;

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::storage::LocalDb;
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};

pub struct AddMemoryTool {
    db: LocalDb,
}

impl AddMemoryTool {
    pub fn new(db: LocalDb) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Tool for AddMemoryTool {
    fn name(&self) -> &str {
        "add_memory"
    }

    fn description(&self) -> &str {
        "Save one explicit long-term memory to Atlas local SQLite."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "add_memory".to_string(),
            description: "Persist a concise user preference or fact. Only call when the user asks to remember something or clearly states a durable preference.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Concise memory text"
                    },
                    "source": {
                        "type": "string",
                        "description": "Memory source label, default agent"
                    }
                },
                "required": ["text"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "保存长期记忆".to_string(),
            description_zh: "把用户明确要求记住的偏好或事实保存到本地 SQLite。".to_string(),
            capability_labels_zh: vec!["记忆".to_string(), "本地数据".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::Memory, ToolCapability::LocalData],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let text = args["text"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("缺少 text 参数。".to_string()))?;
        let source = args
            .get("source")
            .and_then(|value| value.as_str())
            .unwrap_or("agent");
        let memory = self
            .db
            .add_memory(text, source)
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(ToolResult::success(
            "长期记忆已保存到本地 SQLite。",
            serde_json::json!({ "memory": memory }),
        ))
    }
}
