use async_trait::async_trait;

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::storage::LocalDb;
use crate::tools::checkpoint;
use crate::tools::fs_scope;
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};
use std::path::PathBuf;

pub struct EditFileTool {
    db: LocalDb,
    extra_roots: Vec<PathBuf>,
    current_session_id: Option<String>,
}

impl EditFileTool {
    pub fn new(db: LocalDb) -> Self {
        Self::new_with_roots(db, Vec::new(), None)
    }

    pub fn new_with_roots(
        db: LocalDb,
        extra_roots: Vec<PathBuf>,
        current_session_id: Option<String>,
    ) -> Self {
        Self {
            db,
            extra_roots: fs_scope::normalize_extra_roots(extra_roots),
            current_session_id,
        }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit an existing local text file by replacing exact text or a line range. Prefer this for fixes to existing code or HTML."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Edit an existing local text file. Prefer oldText/newText for precise edits; use startLine/endLine/replacement for line ranges. Use this instead of write_file when only part of an existing file needs to change.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Target text file path"
                    },
                    "oldText": {
                        "type": "string",
                        "description": "Exact text to replace once"
                    },
                    "newText": {
                        "type": "string",
                        "description": "Replacement text for oldText"
                    },
                    "startLine": {
                        "type": "integer",
                        "description": "1-based start line for range replacement"
                    },
                    "endLine": {
                        "type": "integer",
                        "description": "1-based inclusive end line for range replacement"
                    },
                    "replacement": {
                        "type": "string",
                        "description": "Replacement text for the line range"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Short reason recorded locally"
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
            label_zh: "编辑文件".to_string(),
            description_zh: "在允许路径内精确编辑文本文件。".to_string(),
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
        let path = fs_scope::allowed_file_with_roots(path, &self.extra_roots)?;
        let original = std::fs::read_to_string(&path)
            .map_err(|error| AgentError::Tool(format!("无法读取待编辑文件: {error}")))?;
        let edited = if let Some(old_text) = args.get("oldText").and_then(|value| value.as_str()) {
            let new_text = args
                .get("newText")
                .and_then(|value| value.as_str())
                .ok_or_else(|| AgentError::Tool("使用 oldText 时必须提供 newText。".to_string()))?;
            if old_text.is_empty() {
                return Err(AgentError::Tool("oldText 不能为空。".to_string()));
            }
            if !original.contains(old_text) {
                return Err(AgentError::Tool("没有找到要替换的精确文本。".to_string()));
            }
            original.replacen(old_text, new_text, 1)
        } else {
            replace_line_range(&original, &args)?
        };

        let reason = args
            .get("reason")
            .and_then(|value| value.as_str())
            .unwrap_or("Agent 编辑本地文本文件。");

        // Capture before-state checkpoint (M4.2). The file definitely exists here
        // (we read it above), so capture is meaningful unless there's no active task.
        let checkpoint_outcome =
            checkpoint::capture_before_write(&self.db, self.current_session_id.as_deref(), &path)
                .map_err(|e| e.into_agent_error())?;

        let pending = self
            .db
            .prepare_file_write(path.clone(), edited, reason.to_string())
            .map_err(|error| AgentError::Tool(error.to_string()))?;
        let preview = self
            .db
            .confirm_pending_file_write(&pending.id, None)
            .map_err(|error| AgentError::Tool(error.to_string()))?;
        let checkpoint_warning = checkpoint::record_after_write(
            &self.db,
            &checkpoint_outcome,
            &PathBuf::from(&preview.target_path),
        )
        .err()
        .map(|e| e.to_string());

        if let Some(message) = checkpoint_warning {
            return Ok(ToolResult::warning(
                format!(
                    "已编辑文件：{}（checkpoint 写后指纹记录失败）",
                    preview.target_path
                ),
                serde_json::json!({
                    "fileEdit": preview,
                    "confirmed": true,
                    "checkpointWarning": message,
                }),
                vec![
                    "不要依赖本次编辑的 reset_task 冲突检测，先复查 checkpoint 存储状态。"
                        .to_string(),
                ],
            ));
        }
        Ok(ToolResult::success(
            format!("已编辑文件：{}", preview.target_path),
            serde_json::json!({ "fileEdit": preview, "confirmed": true }),
        ))
    }
}

fn replace_line_range(original: &str, args: &serde_json::Value) -> Result<String, AgentError> {
    let start = args
        .get("startLine")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| {
            AgentError::Tool("必须提供 oldText 或 startLine/endLine/replacement。".to_string())
        })? as usize;
    let end = args
        .get("endLine")
        .and_then(|value| value.as_u64())
        .unwrap_or(start as u64) as usize;
    let replacement = args
        .get("replacement")
        .and_then(|value| value.as_str())
        .ok_or_else(|| AgentError::Tool("使用行范围编辑时必须提供 replacement。".to_string()))?;
    if start == 0 || end < start {
        return Err(AgentError::Tool("行号范围不合法。".to_string()));
    }
    let line_ending = detect_line_ending(original);
    let mut lines = split_text_lines(original, line_ending);
    if end > lines.len() {
        return Err(AgentError::Tool("行号超过文件总行数。".to_string()));
    }
    let replacement_lines = split_text_lines(replacement, detect_line_ending(replacement));
    lines.splice(start - 1..end, replacement_lines);
    let mut next = lines.join(line_ending);
    if has_trailing_line_ending(original) {
        next.push_str(line_ending);
    }
    Ok(next)
}

fn detect_line_ending(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\r') {
        "\r"
    } else {
        "\n"
    }
}

fn has_trailing_line_ending(text: &str) -> bool {
    text.ends_with('\n') || text.ends_with('\r')
}

fn split_text_lines(text: &str, line_ending: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines = match line_ending {
        "\r\n" => text.split("\r\n").map(str::to_string).collect::<Vec<_>>(),
        "\r" => text.split('\r').map(str::to_string).collect::<Vec<_>>(),
        _ => text.split('\n').map(str::to_string).collect::<Vec<_>>(),
    };
    if has_trailing_line_ending(text) {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        let path = std::env::temp_dir().join(format!("aura_edit_file_test_{}.db", Uuid::new_v4()));
        LocalDb::open(path).unwrap()
    }

    #[tokio::test]
    async fn line_range_edit_preserves_crlf_line_endings() {
        let target = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("aura_edit_tool_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, "one\r\ntwo\r\nthree\r\n").unwrap();
        let tool = EditFileTool::new(temp_db());

        let result = tool
            .execute(serde_json::json!({
                "path": target.to_string_lossy(),
                "startLine": 2,
                "endLine": 2,
                "replacement": "TWO",
                "reason": "test"
            }))
            .await
            .unwrap();

        assert!(matches!(
            result.status,
            crate::agent::ToolResultStatus::Success
        ));
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "one\r\nTWO\r\nthree\r\n"
        );
        let _ = std::fs::remove_file(target);
    }

    #[tokio::test]
    async fn line_range_edit_preserves_cr_only_line_endings() {
        let target = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("aura_edit_tool_cr_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, "one\rtwo\rthree\r").unwrap();
        let tool = EditFileTool::new(temp_db());

        let result = tool
            .execute(serde_json::json!({
                "path": target.to_string_lossy(),
                "startLine": 2,
                "endLine": 2,
                "replacement": "TWO",
                "reason": "test"
            }))
            .await
            .unwrap();

        assert!(matches!(
            result.status,
            crate::agent::ToolResultStatus::Success
        ));
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "one\rTWO\rthree\r"
        );
        let _ = std::fs::remove_file(target);
    }

    #[tokio::test]
    async fn empty_line_range_replacement_deletes_target_line() {
        let target = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("aura_edit_tool_delete_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, "one\ntwo\nthree\n").unwrap();
        let tool = EditFileTool::new(temp_db());

        tool.execute(serde_json::json!({
            "path": target.to_string_lossy(),
            "startLine": 2,
            "endLine": 2,
            "replacement": "",
            "reason": "test"
        }))
        .await
        .unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "one\nthree\n");
        let _ = std::fs::remove_file(target);
    }
}
