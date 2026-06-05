use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::tools::fs_scope;
use crate::tools::output_limit::{truncate_middle, MAX_TOOL_OUTPUT_CHARS};
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};
use async_trait::async_trait;
use std::path::PathBuf;

/// P1-5: hard ceiling for a caller-requested `max_chars` override, so the
/// "expand" path can ask for more of a large file but still can't blow the
/// context wide open.
const READ_FILE_HARD_MAX_CHARS: usize = 200_000;

pub struct ReadFileTool {
    extra_roots: Vec<PathBuf>,
}

impl ReadFileTool {
    pub fn new(extra_roots: Vec<PathBuf>) -> Self {
        Self {
            extra_roots: fs_scope::normalize_extra_roots(extra_roots),
        }
    }
}

impl Default for ReadFileTool {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "read_file".to_string(),
            description: "Read the contents of a file".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path to the file to read"
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Optional cap on returned characters (default 16000). Large files are truncated keeping head + tail; raise this to read more."
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
            label_zh: "读取文件".to_string(),
            description_zh: "读取用户允许路径中的文本文件内容。".to_string(),
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

        let max_chars = args
            .get("max_chars")
            .and_then(|value| value.as_u64())
            .map(|value| (value as usize).clamp(1_000, READ_FILE_HARD_MAX_CHARS))
            .unwrap_or(MAX_TOOL_OUTPUT_CHARS);

        let path = fs_scope::allowed_file_with_roots(path, &self.extra_roots)?;
        let content = std::fs::read_to_string(&path)
            .map_err(|e| AgentError::Tool(format!("Failed to read file: {}", e)))?;
        let path = path.to_string_lossy().to_string();

        // P1-5: bound the returned content so a large file can't blow up the
        // context; keep head + tail and report how much was elided.
        let out = truncate_middle(&content, max_chars);
        let summary = if out.truncated {
            format!(
                "已读取文件 {path}（内容过大，已保留首尾 {} / 共 {} 字符，可用 max_chars 读取更多）",
                out.kept_chars, out.original_chars
            )
        } else {
            format!("已读取文件 {path}")
        };

        Ok(ToolResult::success(
            summary,
            serde_json::json!({
                "path": path,
                "content": out.text,
                "truncated": out.truncated,
                "truncation": out.meta(),
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_existing_file() {
        let tool = ReadFileTool::default();
        let args = serde_json::json!({
            "path": "Cargo.toml"
        });
        let result = tool.execute(args).await;
        assert!(result.is_ok());
        assert!(result.unwrap().data["content"]
            .as_str()
            .unwrap()
            .contains("[package]"));
    }

    #[tokio::test]
    async fn test_read_nonexistent_file() {
        let tool = ReadFileTool::default();
        let args = serde_json::json!({
            "path": "nonexistent_file.txt"
        });
        let result = tool.execute(args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_rejects_existing_file_outside_allowed_scope() {
        let tool = ReadFileTool::default();
        let temp =
            std::env::temp_dir().join(format!("atlas_read_scope_{}.txt", std::process::id()));
        std::fs::write(&temp, "secret").unwrap();
        let result = tool
            .execute(serde_json::json!({ "path": temp.to_string_lossy() }))
            .await;
        std::fs::remove_file(&temp).ok();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn large_file_content_is_truncated_with_head_and_tail() {
        // P1-5: a large file must not return unbounded content; head + tail are kept
        // and the truncation metadata reports the real original size. Write under the
        // build dir (in scope, gitignored, not sensitive) — the OS temp dir on Windows
        // lives under AppData\Local and is treated as a sensitive path.
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("atlas_read_trunc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("big.txt");
        let body = format!("HEAD_MARKER\n{}\nTAIL_MARKER", "x".repeat(50_000));
        std::fs::write(&file, &body).unwrap();

        let tool = ReadFileTool::default();
        let result = tool
            .execute(serde_json::json!({ "path": file.to_string_lossy() }))
            .await
            .unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.data["truncated"], serde_json::json!(true));
        let content = result.data["content"].as_str().unwrap();
        assert!(content.starts_with("HEAD_MARKER"), "head kept");
        assert!(content.ends_with("TAIL_MARKER"), "tail kept");
        assert!(
            content.chars().count() <= MAX_TOOL_OUTPUT_CHARS,
            "content bounded to the cap"
        );
        assert_eq!(
            result.data["truncation"]["originalChars"],
            serde_json::json!(body.chars().count())
        );
    }
}
