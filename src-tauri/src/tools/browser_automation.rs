use async_trait::async_trait;

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::browser_automation::{run_browser_automation, BrowserAutomationRequest};
use crate::storage::LocalDb;
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};

pub struct BrowserAutomationTool {
    db: LocalDb,
    session_id: Option<String>,
    run_id: Option<String>,
}

impl BrowserAutomationTool {
    pub fn new(db: LocalDb, session_id: Option<String>, run_id: Option<String>) -> Self {
        Self {
            db,
            session_id,
            run_id,
        }
    }
}

#[async_trait]
impl Tool for BrowserAutomationTool {
    fn name(&self) -> &str {
        "browser_automation"
    }

    fn description(&self) -> &str {
        "Run a confirmed browser automation action through Atlas's audit boundary."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Use the local Chrome/Playwright browser automation bridge for search, open, screenshot, click, type, or key press. Click/type/press require confirmed=true after explicit user confirmation.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["search", "open", "screenshot", "click", "type", "press"],
                        "description": "Browser action"
                    },
                    "target": {
                        "type": "string",
                        "description": "Known target for search: baidu, bilibili, douyin, kuaishou, zhihu, xiaohongshu, weibo"
                    },
                    "keyword": { "type": "string", "description": "Search keyword" },
                    "url": { "type": "string", "description": "URL for open or screenshot" },
                    "selector": { "type": "string", "description": "CSS selector for click/type" },
                    "text": { "type": "string", "description": "Text for type action" },
                    "key": { "type": "string", "description": "Keyboard key for press action" },
                    "confirmed": { "type": "boolean", "description": "Whether the user confirmed a write-like browser action" },
                    "headless": { "type": "boolean", "description": "Run without showing the browser window, default true" }
                },
                "required": ["action"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "浏览器自动化".to_string(),
            description_zh:
                "通过本机 Chrome/Playwright 执行搜索、打开、截图和经确认的点击输入，并写入审计。"
                    .to_string(),
            capability_labels_zh: vec![
                "浏览器".to_string(),
                "网络".to_string(),
                "需确认".to_string(),
            ],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::Network, ToolCapability::System],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: true,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let mut request: BrowserAutomationRequest =
            serde_json::from_value(args).map_err(|error| AgentError::Tool(error.to_string()))?;
        if request.session_id.is_none() {
            request.session_id = self.session_id.clone();
        }
        if request.run_id.is_none() {
            request.run_id = self.run_id.clone();
        }
        let result = run_browser_automation(&self.db, request)
            .await
            .map_err(AgentError::Tool)?;
        Ok(ToolResult::success(
            if result.ok {
                "浏览器自动化已完成。".to_string()
            } else {
                "浏览器自动化失败。".to_string()
            },
            serde_json::json!(result),
        ))
    }
}
