use async_trait::async_trait;

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::tools::fs_scope;
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};
use std::path::PathBuf;

pub struct CreateDirectoryTool {
    extra_roots: Vec<PathBuf>,
}

impl CreateDirectoryTool {
    pub fn new(extra_roots: Vec<PathBuf>) -> Self {
        Self {
            extra_roots: fs_scope::normalize_extra_roots(extra_roots),
        }
    }
}

impl Default for CreateDirectoryTool {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[async_trait]
impl Tool for CreateDirectoryTool {
    fn name(&self) -> &str {
        "create_directory"
    }

    fn description(&self) -> &str {
        "Create a directory under an allowed local path."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Create a local directory under an allowed project or user folder."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to create"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "创建目录".to_string(),
            description_zh: "在允许路径内创建文件夹。".to_string(),
            capability_labels_zh: vec!["文件系统".to_string(), "写入".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::Filesystem],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("缺少 path 参数。".to_string()))?;
        let target = fs_scope::allowed_new_directory_with_roots(path, &self.extra_roots)?;
        std::fs::create_dir_all(&target)
            .map_err(|error| AgentError::Tool(format!("创建目录失败: {error}")))?;
        Ok(ToolResult::success(
            format!("已创建目录：{}", target.to_string_lossy()),
            serde_json::json!({ "path": target.to_string_lossy(), "created": true }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn creates_nested_directory_under_allowed_scope() {
        let base = std::env::current_dir().unwrap().join("target");
        std::fs::create_dir_all(&base).unwrap();
        let target = base
            .join(format!("atlas_create_dir_{}", Uuid::new_v4()))
            .join("nested")
            .join("page");
        let tool = CreateDirectoryTool::default();

        let result = tool
            .execute(serde_json::json!({ "path": target.to_string_lossy() }))
            .await
            .unwrap();

        assert!(matches!(
            result.status,
            crate::agent::ToolResultStatus::Success
        ));
        assert!(target.is_dir());
        let root = target
            .ancestors()
            .nth(2)
            .expect("target includes generated root")
            .to_path_buf();
        let _ = std::fs::remove_dir_all(root);
    }
}
