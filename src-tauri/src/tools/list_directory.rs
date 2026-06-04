use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::tools::fs_scope;
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};
use async_trait::async_trait;
use std::path::PathBuf;

pub struct ListDirectoryTool {
    extra_roots: Vec<PathBuf>,
}

impl ListDirectoryTool {
    pub fn new(extra_roots: Vec<PathBuf>) -> Self {
        Self {
            extra_roots: fs_scope::normalize_extra_roots(extra_roots),
        }
    }
}

impl Default for ListDirectoryTool {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List files and directories in a directory"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "list_directory".to_string(),
            description: "List files and directories in a directory".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path to the directory to list"
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
            label_zh: "列出目录".to_string(),
            description_zh: "列出用户允许目录中的文件和子目录。".to_string(),
            capability_labels_zh: vec!["文件系统".to_string(), "只读".to_string()],
            safety_label_zh: "安全".to_string(),
            capabilities: vec![ToolCapability::Filesystem, ToolCapability::ReadOnly],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: false,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("缺少 path 参数。".to_string()))?;

        let path = fs_scope::allowed_directory_with_roots(path, &self.extra_roots)?;
        let entries = std::fs::read_dir(&path)
            .map_err(|e| AgentError::Tool(format!("Failed to read directory: {}", e)))?;

        let mut result = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| AgentError::Tool(e.to_string()))?;
            let file_name = entry.file_name().to_string_lossy().to_string();
            let file_type = if entry.path().is_dir() { "dir" } else { "file" };
            result.push(serde_json::json!({ "name": file_name, "type": file_type }));
        }
        let path = path.to_string_lossy().to_string();

        Ok(ToolResult::success(
            format!("已列出目录 {}", path),
            serde_json::json!({ "path": path, "entries": result }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_list_current_directory() {
        let tool = ListDirectoryTool::default();
        let args = serde_json::json!({
            "path": "."
        });
        let result = tool.execute(args).await;
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.data["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "Cargo.toml" || item["name"] == "src"));
    }

    #[tokio::test]
    async fn test_list_nonexistent_directory() {
        let tool = ListDirectoryTool::default();
        let args = serde_json::json!({
            "path": "nonexistent_directory"
        });
        let result = tool.execute(args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_rejects_existing_directory_outside_allowed_scope() {
        let tool = ListDirectoryTool::default();
        let temp = std::env::temp_dir().join(format!("aura_list_scope_{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        let result = tool
            .execute(serde_json::json!({ "path": temp.to_string_lossy() }))
            .await;
        std::fs::remove_dir_all(&temp).ok();
        assert!(result.is_err());
    }
}
