use async_trait::async_trait;

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::tools::fs_scope;
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};
use std::path::PathBuf;

pub struct FileInfoTool {
    extra_roots: Vec<PathBuf>,
}

impl FileInfoTool {
    pub fn new(extra_roots: Vec<PathBuf>) -> Self {
        Self {
            extra_roots: fs_scope::normalize_extra_roots(extra_roots),
        }
    }
}

impl Default for FileInfoTool {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[async_trait]
impl Tool for FileInfoTool {
    fn name(&self) -> &str {
        "file_info"
    }

    fn description(&self) -> &str {
        "Read metadata for an allowed local file or directory."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Read metadata for an allowed local file or directory without reading full contents.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File or directory path"
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
            label_zh: "查看文件信息".to_string(),
            description_zh: "读取允许路径中文件或目录的大小、类型和时间信息。".to_string(),
            capability_labels_zh: vec!["文件系统".to_string(), "只读".to_string()],
            safety_label_zh: "安全".to_string(),
            capabilities: vec![ToolCapability::Filesystem, ToolCapability::ReadOnly],
            safety_level: ToolSafetyLevel::Safe,
            mutates_state: false,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("缺少 path 参数。".to_string()))?;
        let path = fs_scope::allowed_existing_with_roots(path, &self.extra_roots)?;
        let metadata = std::fs::metadata(&path)
            .map_err(|error| AgentError::Tool(format!("无法读取文件信息: {error}")))?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64);

        Ok(ToolResult::success(
            "已读取文件信息。",
            serde_json::json!({
                "path": path.to_string_lossy(),
                "isFile": metadata.is_file(),
                "isDirectory": metadata.is_dir(),
                "sizeBytes": metadata.len(),
                "modifiedAt": modified
            }),
        ))
    }
}
