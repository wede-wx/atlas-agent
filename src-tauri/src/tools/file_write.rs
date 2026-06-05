use async_trait::async_trait;

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::storage::LocalDb;
use crate::tools::checkpoint;
use crate::tools::fs_scope;
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};
use std::path::PathBuf;

pub struct PrepareFileWriteTool {
    db: LocalDb,
    extra_roots: Vec<PathBuf>,
}

pub struct WriteFileTool {
    db: LocalDb,
    extra_roots: Vec<PathBuf>,
    current_session_id: Option<String>,
}

impl PrepareFileWriteTool {
    pub fn new(db: LocalDb) -> Self {
        Self::new_with_roots(db, Vec::new())
    }

    pub fn new_with_roots(db: LocalDb, extra_roots: Vec<PathBuf>) -> Self {
        Self {
            db,
            extra_roots: fs_scope::normalize_extra_roots(extra_roots),
        }
    }
}

impl WriteFileTool {
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
impl Tool for PrepareFileWriteTool {
    fn name(&self) -> &str {
        "prepare_file_write"
    }

    fn description(&self) -> &str {
        "Prepare a local UTF-8 text file write for confirmation mode. It never writes to disk until the user confirms."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Prepare a local UTF-8 text file write preview. Do not call this for deletion, moving files, or binary writes.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or user-home-relative target path for a UTF-8 text file, including code and web files such as .html, .css, and .js"
                    },
                    "content": {
                        "type": "string",
                        "description": "Text content to write"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Short reason shown to the user"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "准备写入文件".to_string(),
            description_zh: "生成写入预览，必须用户确认后才会真正写盘。".to_string(),
            capability_labels_zh: vec!["文件系统".to_string(), "需确认".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::Filesystem, ToolCapability::System],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: false,
            requires_confirmation: true,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult, AgentError> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("Missing 'path' parameter".to_string()))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("Missing 'content' parameter".to_string()))?;
        let reason = args
            .get("reason")
            .and_then(|value| value.as_str())
            .unwrap_or("Agent 准备写入本地文件。");

        let path = fs_scope::allowed_new_path_with_roots(path, &self.extra_roots)?;
        let preview = self
            .db
            .prepare_file_write(path, content.to_string(), reason.to_string())
            .map_err(|e| AgentError::Tool(e.to_string()))?;

        // P0-1: flag secrets in the previewed content so the user sees the risk
        // before confirming the write (content is left intact for the preview).
        let secret_report = crate::tools::secret_scan::scan(
            content,
            crate::tools::secret_scan::SecretLocation::FileWrite,
            crate::tools::secret_scan::SecretAction::AllowedWithWarning,
        );
        let mut data = serde_json::json!({
            "pendingWrite": preview,
            "confirmed": false,
            "playbackState": "not_applicable"
        });
        let mut next_actions = vec![
            "向用户说明目标路径、是否覆盖和内容摘要。".to_string(),
            "提醒用户必须手动确认后才会写入文件。".to_string(),
        ];
        if secret_report.has_secrets() {
            data["secretFindings"] =
                serde_json::to_value(&secret_report.findings).unwrap_or_default();
            next_actions.push(format!(
                "内容含 {} 处疑似密钥，提醒用户确认是否写入明文。",
                secret_report.findings.len()
            ));
        }
        Ok(ToolResult::warning(
            "已生成文件写入预览，尚未写盘。请在确认队列里查看并确认或拒绝。",
            data,
            next_actions,
        ))
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write a new local UTF-8 text file immediately, or replace a whole file only when explicitly needed."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Immediately write a local UTF-8 text file, including code and web files such as .html, .css, and .js. Use edit_file for small changes to existing files; use write_file for new files or explicit full replacement only. Do not call this for deletion, moving files, or binary writes.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or user-home-relative target path for a UTF-8 text file, including code and web files such as .html, .css, and .js"
                    },
                    "content": {
                        "type": "string",
                        "description": "Text content to write"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Short reason recorded in local activity"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "直接写入文件".to_string(),
            description_zh: "按当前权限直接写入用户明确指定的本地文本、代码或网页文件。"
                .to_string(),
            capability_labels_zh: vec!["文件系统".to_string(), "直接执行".to_string()],
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
            .ok_or_else(|| AgentError::Tool("Missing 'path' parameter".to_string()))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("Missing 'content' parameter".to_string()))?;
        let reason = args
            .get("reason")
            .and_then(|value| value.as_str())
            .unwrap_or("Agent 直接写入本地文件。");

        let path = fs_scope::allowed_new_path_with_roots(path, &self.extra_roots)?;

        // Capture before-state checkpoint (M4.2). Failure here aborts the write so
        // we never mutate disk without a recoverable rollback record.
        let checkpoint_outcome =
            checkpoint::capture_before_write(&self.db, self.current_session_id.as_deref(), &path)
                .map_err(|e| e.into_agent_error())?;

        let pending = self
            .db
            .prepare_file_write(path, content.to_string(), reason.to_string())
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        let preview = self
            .db
            .confirm_pending_file_write(&pending.id, None)
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        let checkpoint_warning = checkpoint::record_after_write(
            &self.db,
            &checkpoint_outcome,
            &PathBuf::from(&preview.target_path),
        )
        .err()
        .map(|e| e.to_string());

        // P0-1: scan the written content for secrets. We do not alter the bytes —
        // that would corrupt an intended write — but surface a warning and an
        // auditable finding so plaintext credentials never land silently.
        let secret_report = crate::tools::secret_scan::scan(
            content,
            crate::tools::secret_scan::SecretLocation::FileWrite,
            crate::tools::secret_scan::SecretAction::AllowedWithWarning,
        );
        let mut data = serde_json::json!({
            "fileWrite": preview,
            "confirmed": true,
            "playbackState": "not_applicable"
        });
        if let Some(message) = checkpoint_warning.as_deref() {
            data["checkpointWarning"] = serde_json::json!(message);
        }
        if secret_report.has_secrets() || checkpoint_warning.is_some() {
            let mut warnings = Vec::new();
            let mut next_actions = Vec::new();
            data["secretFindings"] =
                serde_json::to_value(&secret_report.findings).unwrap_or_default();
            if secret_report.has_secrets() {
                warnings.push(format!(
                    "检测到 {} 处疑似密钥",
                    secret_report.findings.len()
                ));
                next_actions
                    .push("提示用户文件含疑似密钥，建议改用环境变量或密钥管理。".to_string());
            }
            if checkpoint_warning.is_some() {
                warnings.push("checkpoint 写后指纹记录失败".to_string());
                next_actions.push(
                    "不要依赖本次写入的 reset_task 冲突检测，先复查 checkpoint 存储状态。"
                        .to_string(),
                );
            }
            return Ok(ToolResult::warning(
                format!(
                    "已写入文件：{}（{}）",
                    preview.target_path,
                    warnings.join("；")
                ),
                data,
                next_actions,
            ));
        }
        Ok(ToolResult::success(
            format!("已写入文件：{}", preview.target_path),
            data,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        let path =
            std::env::temp_dir().join(format!("atlas_file_write_test_{}.db", Uuid::new_v4()));
        LocalDb::open(path).unwrap()
    }

    fn setup_session_with_active_task(db: &LocalDb) -> (String, String) {
        let session = db.create_session("write-checkpoint-test").unwrap();
        let task = db
            .create_plan_task_full(&session.id, "write file", None, None, "test", None, None)
            .unwrap();
        db.set_active_plan_task(&session.id, Some(&task.id))
            .unwrap();
        (session.id, task.id)
    }

    #[tokio::test]
    async fn write_file_tool_writes_text_immediately() {
        let target = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("atlas_write_tool_{}.txt", Uuid::new_v4()));
        let tool = WriteFileTool::new(temp_db());
        let result = tool
            .execute(serde_json::json!({
                "path": target.to_string_lossy(),
                "content": "atlas write tool test",
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
            "atlas write tool test"
        );
        let _ = std::fs::remove_file(target);
    }

    #[tokio::test]
    async fn write_file_tool_records_after_hash_for_checkpoint_conflict_baseline() {
        let db = temp_db();
        let (session_id, task_id) = setup_session_with_active_task(&db);
        let target = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("atlas_write_checkpoint_{}.txt", Uuid::new_v4()));
        let tool = WriteFileTool::new_with_roots(db.clone(), Vec::new(), Some(session_id));

        let result = tool
            .execute(serde_json::json!({
                "path": target.to_string_lossy(),
                "content": "checkpoint baseline",
                "reason": "test"
            }))
            .await
            .unwrap();

        assert!(matches!(
            result.status,
            crate::agent::ToolResultStatus::Success
        ));
        let checkpoints = db.list_file_checkpoints(&task_id).unwrap();
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(
            checkpoints[0].before_hash.as_deref(),
            Some(checkpoint::MISSING_FILE_SENTINEL)
        );
        assert!(
            checkpoints[0]
                .after_hash
                .as_deref()
                .is_some_and(|hash| hash.starts_with("sha256:") && hash.len() == 71),
            "write_file must persist the post-write hash used by reset conflict detection"
        );
        assert_eq!(
            checkpoints[0].after_content.as_deref(),
            Some("checkpoint baseline"),
            "write_file must persist the post-write text snapshot used by run diff"
        );
        let _ = std::fs::remove_file(target);
    }

    #[tokio::test]
    async fn write_file_tool_writes_html_without_extension_block() {
        let target = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("atlas_write_tool_{}.html", Uuid::new_v4()));
        let html =
            "<!doctype html><html><body><script>console.log('atlas');</script></body></html>";
        let tool = WriteFileTool::new(temp_db());
        let result = tool
            .execute(serde_json::json!({
                "path": target.to_string_lossy(),
                "content": html,
                "reason": "test"
            }))
            .await
            .unwrap();

        assert!(matches!(
            result.status,
            crate::agent::ToolResultStatus::Success
        ));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), html);
        let _ = std::fs::remove_file(target);
    }

    #[tokio::test]
    async fn write_file_tool_warns_when_content_contains_secret() {
        // Proves the P0-1 secret scan point is actually wired into write_file,
        // not just present as a standalone module.
        let target = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("atlas_write_secret_{}.txt", Uuid::new_v4()));
        let tool = WriteFileTool::new(temp_db());
        let result = tool
            .execute(serde_json::json!({
                "path": target.to_string_lossy(),
                "content": "aws_key = AKIAIOSFODNN7EXAMPLE",
                "reason": "test"
            }))
            .await
            .unwrap();

        assert!(
            matches!(result.status, crate::agent::ToolResultStatus::Warning),
            "writing a secret should downgrade the result to a warning"
        );
        assert!(
            result.data.get("secretFindings").is_some(),
            "the warning must carry the secret findings"
        );
        let _ = std::fs::remove_file(target);
    }

    #[tokio::test]
    async fn write_file_tool_rejects_paths_outside_allowed_scope() {
        let target =
            std::env::temp_dir().join(format!("atlas_write_tool_reject_{}.txt", Uuid::new_v4()));
        let tool = WriteFileTool::new(temp_db());
        let result = tool
            .execute(serde_json::json!({
                "path": target.to_string_lossy(),
                "content": "should not write",
                "reason": "test"
            }))
            .await;

        assert!(result.is_err());
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn prepare_file_write_tool_rejects_paths_outside_allowed_scope() {
        let target =
            std::env::temp_dir().join(format!("atlas_prepare_tool_reject_{}.txt", Uuid::new_v4()));
        let tool = PrepareFileWriteTool::new(temp_db());
        let result = tool
            .execute(serde_json::json!({
                "path": target.to_string_lossy(),
                "content": "should not prepare",
                "reason": "test"
            }))
            .await;

        assert!(result.is_err());
        assert!(!target.exists());
    }
}
