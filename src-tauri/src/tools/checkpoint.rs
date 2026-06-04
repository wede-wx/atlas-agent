//! File checkpoint capture / restore (M4.2 of millimeter plan).
//!
//! `capture_before_write` is called by `write_file` / `edit_file` before any
//! actual disk mutation. It records the original file content into
//! `run_file_checkpoints` so the active task can later be rolled back.
//!
//! Tiered storage:
//! - file does not exist before write → record a "missing" marker (reset deletes the file)
//! - size ≤ INLINE_LIMIT_BYTES + valid UTF-8 → inline into DB `before_content`
//! - otherwise → write blob to `~/.aura/checkpoints/<run_id_or_session>/<task_id>/<hash>.before`
//!   and store the blob path in DB
//! - size > HARD_LIMIT_BYTES → reject the write entirely (recoverable error)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::storage::{aura_home, FileCheckpointRecord, LocalDb};
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};

pub const INLINE_LIMIT_BYTES: u64 = 64 * 1024;
pub const HARD_LIMIT_BYTES: u64 = 5 * 1024 * 1024;
const MAX_DIFF_TEXT_CHARS: usize = 64 * 1024;
const MAX_DIFF_LINES: usize = 900;
const MAX_DIFF_LCS_CELLS: usize = 1_000_000;

pub const MISSING_FILE_SENTINEL: &str = "__missing_before_write__";

/// M-9 (d): the checkpoint contract. Tools that overwrite the *content* of an
/// existing file must call [`capture_before_write`] before the write and
/// [`record_after_write`] after it, so every content mutation is reversible and
/// surfaces in the run diff. This list is the single source of truth; the
/// registry tripwire test `file_content_writers_honor_checkpoint_contract`
/// fails if a new filesystem-mutating tool is registered without being
/// classified, forcing a conscious decision about checkpoint coverage.
///
/// NOT included (they mutate the filesystem but not existing file content, so
/// the before/after-content contract does not apply): `create_directory`
/// (creates an empty dir), `reset_task` / `purge_run_checkpoints` (the rollback
/// side), and the `git_*` write tools (mutate git state, audited under P3-1).
pub const FILE_CONTENT_WRITE_TOOLS: &[&str] = &["write_file", "edit_file"];

/// Whether `tool_name` is a file-content writer bound by the checkpoint contract.
pub fn honors_file_checkpoint_contract(tool_name: &str) -> bool {
    FILE_CONTENT_WRITE_TOOLS.contains(&tool_name)
}

#[derive(Debug)]
pub enum CheckpointError {
    FileTooLarge { path: String, size: u64 },
    Io(String),
    Db(String),
}

impl CheckpointError {
    pub fn into_agent_error(self) -> AgentError {
        match self {
            CheckpointError::FileTooLarge { path, size } => AgentError::Tool(format!(
                "目标文件 {path} 大小 {size} 字节，超过 checkpoint 写入上限 {HARD_LIMIT_BYTES} 字节，已拒绝写入。"
            )),
            CheckpointError::Io(msg) => AgentError::Tool(format!("checkpoint IO 失败: {msg}")),
            CheckpointError::Db(msg) => AgentError::Tool(format!("checkpoint 存储失败: {msg}")),
        }
    }
}

impl std::fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckpointError::FileTooLarge { path, size } => write!(
                f,
                "目标文件 {path} 大小 {size} 字节，超过 checkpoint 写入上限 {HARD_LIMIT_BYTES} 字节"
            ),
            CheckpointError::Io(msg) => write!(f, "checkpoint IO 失败: {msg}"),
            CheckpointError::Db(msg) => write!(f, "checkpoint 存储失败: {msg}"),
        }
    }
}

impl std::error::Error for CheckpointError {}

#[derive(Debug, Clone, Copy)]
pub enum CheckpointSkipReason {
    NoSession,
    NoActiveTask,
}

#[derive(Debug)]
pub enum CheckpointOutcome {
    Captured(Box<FileCheckpointRecord>),
    Skipped(CheckpointSkipReason),
}

/// Root for blob storage. `~/.aura/checkpoints` by default; overridable via AURA_HOME.
pub fn checkpoints_root() -> Result<PathBuf, CheckpointError> {
    aura_home()
        .map(|p| p.join("checkpoints"))
        .map_err(|e| CheckpointError::Io(e.to_string()))
}

fn fingerprint(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

fn current_file_fingerprint(path: &Path) -> Result<String, CheckpointError> {
    current_file_snapshot(path).map(|snapshot| snapshot.0)
}

fn current_file_snapshot(path: &Path) -> Result<(String, Option<String>), CheckpointError> {
    if !path.exists() {
        return Ok((MISSING_FILE_SENTINEL.to_string(), None));
    }
    let metadata = std::fs::metadata(path).map_err(|e| CheckpointError::Io(e.to_string()))?;
    let bytes = std::fs::read(path).map_err(|e| CheckpointError::Io(e.to_string()))?;
    let content = if metadata.len() <= INLINE_LIMIT_BYTES {
        std::str::from_utf8(&bytes).ok().map(str::to_string)
    } else {
        None
    };
    Ok((fingerprint(&bytes), content))
}

/// Capture before-state of `target_path`.
///
/// Returns `Skipped` if no session or no active task — callers should NOT treat this as failure,
/// because the active-task gate runs separately (in agent core) and is responsible for enforcing
/// "no active task → reject write". `capture_before_write` is a best-effort safety net.
pub fn capture_before_write(
    db: &LocalDb,
    session_id: Option<&str>,
    target_path: &Path,
) -> Result<CheckpointOutcome, CheckpointError> {
    let Some(sid) = session_id else {
        return Ok(CheckpointOutcome::Skipped(CheckpointSkipReason::NoSession));
    };
    let active = db
        .get_active_plan_task(sid)
        .map_err(|e| CheckpointError::Db(e.to_string()))?;
    let Some(task) = active else {
        return Ok(CheckpointOutcome::Skipped(
            CheckpointSkipReason::NoActiveTask,
        ));
    };

    let path_str = target_path.to_string_lossy().to_string();
    let run_id = task.run_id.clone();

    // Case 1: file does not exist before write — record a sentinel so reset deletes it.
    if !target_path.exists() {
        let record = db
            .record_file_checkpoint(
                &path_str,
                run_id.as_deref(),
                Some(&task.id),
                Some(MISSING_FILE_SENTINEL),
                None,
                None,
                None,
                -1,
            )
            .map_err(|e| CheckpointError::Db(e.to_string()))?;
        return Ok(CheckpointOutcome::Captured(Box::new(record)));
    }

    let metadata =
        std::fs::metadata(target_path).map_err(|e| CheckpointError::Io(e.to_string()))?;
    let size = metadata.len();

    if size > HARD_LIMIT_BYTES {
        return Err(CheckpointError::FileTooLarge {
            path: path_str,
            size,
        });
    }

    let bytes = std::fs::read(target_path).map_err(|e| CheckpointError::Io(e.to_string()))?;
    let hash = fingerprint(&bytes);

    if size <= INLINE_LIMIT_BYTES {
        if let Ok(content) = std::str::from_utf8(&bytes) {
            let record = db
                .record_file_checkpoint(
                    &path_str,
                    run_id.as_deref(),
                    Some(&task.id),
                    Some(&hash),
                    None,
                    Some(content),
                    None,
                    size as i64,
                )
                .map_err(|e| CheckpointError::Db(e.to_string()))?;
            return Ok(CheckpointOutcome::Captured(Box::new(record)));
        }
    }

    // External blob storage.
    let blob_root = checkpoints_root()?;
    let bucket = run_id.clone().unwrap_or_else(|| format!("session-{sid}"));
    let task_dir = blob_root.join(bucket).join(&task.id);
    std::fs::create_dir_all(&task_dir).map_err(|e| CheckpointError::Io(e.to_string()))?;
    let blob_path = task_dir.join(format!("{hash}.before"));
    std::fs::write(&blob_path, &bytes).map_err(|e| CheckpointError::Io(e.to_string()))?;
    let blob_path_str = blob_path.to_string_lossy().to_string();
    let record = db
        .record_file_checkpoint(
            &path_str,
            run_id.as_deref(),
            Some(&task.id),
            Some(&hash),
            None,
            None,
            Some(&blob_path_str),
            size as i64,
        )
        .map_err(|e| CheckpointError::Db(e.to_string()))?;
    Ok(CheckpointOutcome::Captured(Box::new(record)))
}

/// Record the on-disk state after a successful write/edit.
///
/// `after_hash` is the conflict baseline used by `reset_task`: if a later reset
/// sees different bytes, it treats the file as user-modified and refuses to
/// overwrite unless the caller explicitly forces rollback.
/// `after_content` is also stored for small UTF-8 files so run diff views can
/// render historical snapshots without reading the current file from disk.
pub fn record_after_write(
    db: &LocalDb,
    outcome: &CheckpointOutcome,
    target_path: &Path,
) -> Result<(), CheckpointError> {
    let CheckpointOutcome::Captured(record) = outcome else {
        return Ok(());
    };
    let (after_hash, after_content) = current_file_snapshot(target_path)?;
    db.set_file_checkpoint_after_snapshot(&record.id, Some(&after_hash), after_content.as_deref())
        .map_err(|e| CheckpointError::Db(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunDiff {
    pub run_id: String,
    pub total_files: usize,
    pub returned_files: usize,
    pub truncated: bool,
    pub files: Vec<RunDiffFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunDiffFile {
    pub path: String,
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
    pub stats_accurate: bool,
    pub diff_text: Option<String>,
    pub diff_truncated: bool,
    pub unavailable_reason: Option<String>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub first_checkpoint_id: String,
    pub last_checkpoint_id: String,
    pub checkpoint_count: usize,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn build_run_diff(
    db: &LocalDb,
    run_id: &str,
    limit: usize,
) -> Result<RunDiff, CheckpointError> {
    let checkpoints = db
        .list_file_checkpoints_by_run(run_id)
        .map_err(|e| CheckpointError::Db(e.to_string()))?;
    Ok(run_diff_from_checkpoints(run_id, checkpoints, limit))
}

fn run_diff_from_checkpoints(
    run_id: &str,
    checkpoints: Vec<FileCheckpointRecord>,
    limit: usize,
) -> RunDiff {
    let mut by_path = HashMap::<String, Vec<FileCheckpointRecord>>::new();
    for checkpoint in checkpoints {
        by_path
            .entry(checkpoint.path.clone())
            .or_default()
            .push(checkpoint);
    }

    let mut files = by_path
        .into_values()
        .map(run_diff_file_from_group)
        .collect::<Vec<_>>();
    files.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.path.cmp(&b.path))
    });
    let total_files = files.len();
    let max_files = limit.max(1);
    let truncated = files.len() > max_files;
    files.truncate(max_files);
    let returned_files = files.len();
    RunDiff {
        run_id: run_id.to_string(),
        total_files,
        returned_files,
        truncated,
        files,
    }
}

fn run_diff_file_from_group(mut group: Vec<FileCheckpointRecord>) -> RunDiffFile {
    group.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let first = group.first().expect("group is non-empty");
    let last = group.last().expect("group is non-empty");
    let before = before_snapshot(first);
    let after = after_snapshot(last);
    let status = diff_status(first, last);
    let (additions, deletions, stats_accurate, diff_text, diff_truncated, unavailable_reason) =
        diff_payload(&first.path, &before, &after);

    RunDiffFile {
        path: first.path.clone(),
        status,
        additions,
        deletions,
        stats_accurate,
        diff_text,
        diff_truncated,
        unavailable_reason,
        before_hash: first.before_hash.clone(),
        after_hash: last.after_hash.clone(),
        first_checkpoint_id: first.id.clone(),
        last_checkpoint_id: last.id.clone(),
        checkpoint_count: group.len(),
        created_at: first.created_at,
        updated_at: last.created_at,
    }
}

enum TextSnapshot {
    Missing,
    Text(String),
    Unavailable(&'static str),
}

fn before_snapshot(checkpoint: &FileCheckpointRecord) -> TextSnapshot {
    if checkpoint.before_hash.as_deref() == Some(MISSING_FILE_SENTINEL)
        || checkpoint.before_size < 0
    {
        return TextSnapshot::Missing;
    }
    if let Some(content) = checkpoint.before_content.as_ref() {
        return TextSnapshot::Text(content.clone());
    }
    if checkpoint.before_blob_path.is_some() {
        return TextSnapshot::Unavailable("before_content_external_blob");
    }
    TextSnapshot::Unavailable("before_content_missing")
}

fn after_snapshot(checkpoint: &FileCheckpointRecord) -> TextSnapshot {
    match checkpoint.after_hash.as_deref() {
        None => TextSnapshot::Unavailable("after_content_missing"),
        Some(MISSING_FILE_SENTINEL) => TextSnapshot::Missing,
        Some(_) => checkpoint
            .after_content
            .clone()
            .map(TextSnapshot::Text)
            .unwrap_or(TextSnapshot::Unavailable("after_content_missing")),
    }
}

fn diff_status(first: &FileCheckpointRecord, last: &FileCheckpointRecord) -> String {
    let before_missing =
        first.before_hash.as_deref() == Some(MISSING_FILE_SENTINEL) || first.before_size < 0;
    let after_missing = last.after_hash.as_deref() == Some(MISSING_FILE_SENTINEL);
    if before_missing && !after_missing {
        "created".to_string()
    } else if !before_missing && after_missing {
        "deleted".to_string()
    } else if first.before_hash.is_some()
        && last.after_hash.is_some()
        && first.before_hash == last.after_hash
    {
        "unchanged".to_string()
    } else {
        "modified".to_string()
    }
}

fn diff_payload(
    path: &str,
    before: &TextSnapshot,
    after: &TextSnapshot,
) -> (usize, usize, bool, Option<String>, bool, Option<String>) {
    let before_text = match before {
        TextSnapshot::Missing => "",
        TextSnapshot::Text(text) => text.as_str(),
        TextSnapshot::Unavailable(reason) => {
            return (0, 0, false, None, false, Some((*reason).to_string()));
        }
    };
    let after_text = match after {
        TextSnapshot::Missing => "",
        TextSnapshot::Text(text) => text.as_str(),
        TextSnapshot::Unavailable(reason) => {
            return (0, 0, false, None, false, Some((*reason).to_string()));
        }
    };
    let before_lines = split_for_diff(before_text);
    let after_lines = split_for_diff(after_text);
    let cell_count = before_lines.len().saturating_mul(after_lines.len());
    if before_lines.len() > MAX_DIFF_LINES
        || after_lines.len() > MAX_DIFF_LINES
        || cell_count > MAX_DIFF_LCS_CELLS
    {
        return (
            after_lines.len(),
            before_lines.len(),
            false,
            None,
            false,
            Some("diff_too_large".to_string()),
        );
    }
    let (ops, additions, deletions) = line_diff_ops(&before_lines, &after_lines);
    let (diff_text, diff_truncated) = render_unified_diff(path, &ops, &before_lines, &after_lines);
    (
        additions,
        deletions,
        true,
        Some(diff_text),
        diff_truncated,
        None,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffOp {
    Equal(usize),
    Add(usize),
    Delete(usize),
}

fn split_for_diff(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.lines().collect()
    }
}

fn line_diff_ops<'a>(before: &[&'a str], after: &[&'a str]) -> (Vec<DiffOp>, usize, usize) {
    let m = before.len();
    let n = after.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if before[i] == after[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut ops = Vec::new();
    let mut additions = 0usize;
    let mut deletions = 0usize;
    let (mut i, mut j) = (0usize, 0usize);
    while i < m && j < n {
        if before[i] == after[j] {
            ops.push(DiffOp::Equal(i));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ops.push(DiffOp::Delete(i));
            deletions += 1;
            i += 1;
        } else {
            ops.push(DiffOp::Add(j));
            additions += 1;
            j += 1;
        }
    }
    while i < m {
        ops.push(DiffOp::Delete(i));
        deletions += 1;
        i += 1;
    }
    while j < n {
        ops.push(DiffOp::Add(j));
        additions += 1;
        j += 1;
    }
    (ops, additions, deletions)
}

fn render_unified_diff(
    path: &str,
    ops: &[DiffOp],
    before: &[&str],
    after: &[&str],
) -> (String, bool) {
    let mut out = format!("--- a/{path}\n+++ b/{path}\n@@\n");
    let mut truncated = false;
    for op in ops {
        let line = match *op {
            DiffOp::Equal(index) => format!(" {}\n", before[index]),
            DiffOp::Add(index) => format!("+{}\n", after[index]),
            DiffOp::Delete(index) => format!("-{}\n", before[index]),
        };
        if out.len().saturating_add(line.len()) > MAX_DIFF_TEXT_CHARS {
            out.push_str("... diff truncated ...\n");
            truncated = true;
            break;
        }
        out.push_str(&line);
    }
    (out, truncated)
}

// ---------------------------------------------------------------------------
// reset_task tool
// ---------------------------------------------------------------------------

pub struct ResetTaskTool {
    db: LocalDb,
    current_session_id: Option<String>,
}

impl ResetTaskTool {
    pub fn new(db: LocalDb, current_session_id: Option<String>) -> Self {
        Self {
            db,
            current_session_id,
        }
    }
}

#[async_trait]
impl Tool for ResetTaskTool {
    fn name(&self) -> &str {
        "reset_task"
    }

    fn description(&self) -> &str {
        "Roll back the active plan task by restoring all captured file checkpoints to their before-write state."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description:
                "Restore every checkpointed file written during the current active plan task back \
                 to its pre-write contents. Files that did not exist before the task started will be deleted. \
                 The task itself is not auto-marked done; status remains under the model's control."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Optional: plan task id. Defaults to the session's currently active task."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Short reason recorded in local activity."
                    },
                    "force": {
                        "type": "boolean",
                        "description": "If true, restore even when current file fingerprints no longer match the checkpoint after-state. Defaults to false."
                    }
                }
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "回滚任务".to_string(),
            description_zh: "把当前活跃任务期间写过的文件全部恢复到改动前的样子。".to_string(),
            capability_labels_zh: vec!["文件系统".to_string(), "回滚".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::Filesystem, ToolCapability::System],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let explicit_task = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

        let task_id = match explicit_task {
            Some(id) => id,
            None => {
                let sid = self.current_session_id.as_ref().ok_or_else(|| {
                    AgentError::Tool("当前没有绑定会话，无法定位活跃任务。".to_string())
                })?;
                let active = self
                    .db
                    .get_active_plan_task(sid)
                    .map_err(|e| AgentError::Tool(e.to_string()))?;
                active
                    .ok_or_else(|| {
                        AgentError::Tool(
                            "当前没有激活任务，先调 set_active_plan_task 或指定 task_id。"
                                .to_string(),
                        )
                    })?
                    .id
            }
        };

        let outcome =
            perform_reset_task_with_options(&self.db, &task_id, ResetTaskOptions { force })
                .map_err(|e| e.into_agent_error())?;
        let data = json!({
            "taskId": task_id,
            "total": outcome.total,
            "deleted": outcome.deleted,
            "restored": outcome.restored,
            "failed": outcome.failed,
            "conflicts": outcome.conflicts,
            "forcedConflicts": outcome.forced_conflicts,
            "details": outcome.details,
        });
        if outcome.conflicts > 0 {
            return Ok(ToolResult::warning(
                format!(
                    "检测到 {} 个 checkpoint 冲突，已停止覆盖用户期间改动的文件。",
                    outcome.conflicts
                ),
                data,
                vec![
                    "先查看 details 中 result=conflict 的文件，确认这些改动是否应保留。"
                        .to_string(),
                    "如确认要覆盖这些用户期间改动，可再次调用 reset_task 并传 force: true。"
                        .to_string(),
                ],
            ));
        }
        Ok(ToolResult::success(
            format!(
                "已回滚任务 {} 的 {} 个文件 checkpoint（删除 {}，恢复 {}，强制冲突 {}）。",
                task_id, outcome.total, outcome.deleted, outcome.restored, outcome.forced_conflicts
            ),
            data,
        ))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ResetTaskOptions {
    pub force: bool,
}

#[derive(Debug, Default)]
pub struct ResetTaskOutcome {
    pub total: usize,
    pub restored: usize,
    pub deleted: usize,
    pub failed: usize,
    pub conflicts: usize,
    pub forced_conflicts: usize,
    pub details: Vec<Value>,
}

/// Pure logic so the tool and tests can both drive it.
pub fn perform_reset_task(
    db: &LocalDb,
    task_id: &str,
) -> Result<ResetTaskOutcome, CheckpointError> {
    perform_reset_task_with_options(db, task_id, ResetTaskOptions::default())
}

pub fn perform_reset_task_with_options(
    db: &LocalDb,
    task_id: &str,
    options: ResetTaskOptions,
) -> Result<ResetTaskOutcome, CheckpointError> {
    let mut checkpoints = db
        .list_file_checkpoints(task_id)
        .map_err(|e| CheckpointError::Db(e.to_string()))?;
    let mut latest_after_hash_by_path = std::collections::HashMap::<String, Option<String>>::new();
    let mut checkpoint_ids_by_path = std::collections::HashMap::<String, Vec<String>>::new();
    for ckpt in &checkpoints {
        checkpoint_ids_by_path
            .entry(ckpt.path.clone())
            .or_default()
            .push(ckpt.id.clone());
        if ckpt.restored_at.is_none() && !latest_after_hash_by_path.contains_key(&ckpt.path) {
            latest_after_hash_by_path.insert(ckpt.path.clone(), ckpt.after_hash.clone());
        }
    }
    // list_file_checkpoints returns DESC by created_at; we want OLDEST per path
    // (that is the true pre-task state). Reverse so iter is ASC, then keep first per path.
    checkpoints.reverse();
    let mut seen = std::collections::HashSet::<String>::new();
    let mut outcome = ResetTaskOutcome::default();

    for ckpt in &checkpoints {
        if ckpt.restored_at.is_some() {
            seen.insert(ckpt.path.clone());
            continue;
        }
        if !seen.insert(ckpt.path.clone()) {
            // The oldest checkpoint for this path already restored the pre-task state.
            continue;
        }
        outcome.total += 1;
        let target = PathBuf::from(&ckpt.path);
        let expected_after_hash = latest_after_hash_by_path
            .get(&ckpt.path)
            .and_then(|hash| hash.as_deref());
        let detail = restore_with_conflict_check(ckpt, &target, options, expected_after_hash);
        match &detail.kind {
            RestoreKind::Deleted => outcome.deleted += 1,
            RestoreKind::Restored => outcome.restored += 1,
            RestoreKind::Failed => outcome.failed += 1,
            RestoreKind::Conflict => outcome.conflicts += 1,
        }
        if detail.forced_conflict.is_some() && !matches!(detail.kind, RestoreKind::Conflict) {
            outcome.forced_conflicts += 1;
        }
        let mut detail_json = json!({
            "path": ckpt.path,
            "checkpointId": ckpt.id,
            "result": detail.label,
            "message": detail.message,
        });
        let forced = detail.forced_conflict.is_some();
        if let Some(conflict) = detail.conflict.as_ref().or(detail.forced_conflict.as_ref()) {
            detail_json["conflict"] = json!({
                "expectedAfterHash": conflict.expected_after_hash,
                "currentHash": conflict.current_hash,
                "reason": conflict.reason,
                "forced": forced,
            });
        }
        outcome.details.push(detail_json);
        if !matches!(detail.kind, RestoreKind::Failed | RestoreKind::Conflict) {
            if let Some(ids) = checkpoint_ids_by_path.get(&ckpt.path) {
                for checkpoint_id in ids {
                    let _ = db.mark_file_checkpoint_restored(checkpoint_id);
                }
            }
        }
    }

    Ok(outcome)
}

enum RestoreKind {
    Restored,
    Deleted,
    Failed,
    Conflict,
}

struct RestoreDetail {
    kind: RestoreKind,
    label: &'static str,
    message: String,
    conflict: Option<ResetConflict>,
    forced_conflict: Option<ResetConflict>,
}

#[derive(Debug, Clone)]
struct ResetConflict {
    expected_after_hash: String,
    current_hash: String,
    reason: &'static str,
}

impl ResetConflict {
    fn message(&self) -> String {
        match self.reason {
            "missing_after_hash" => {
                "checkpoint has no after-state hash; reset did not overwrite without force"
                    .to_string()
            }
            _ => "current file changed after checkpoint; reset did not overwrite it".to_string(),
        }
    }
}

fn restore_with_conflict_check(
    ckpt: &FileCheckpointRecord,
    target: &Path,
    options: ResetTaskOptions,
    expected_after_hash: Option<&str>,
) -> RestoreDetail {
    match detect_reset_conflict(expected_after_hash, target) {
        Ok(Some(conflict)) if !options.force => RestoreDetail {
            kind: RestoreKind::Conflict,
            label: "conflict",
            message: conflict.message(),
            conflict: Some(conflict),
            forced_conflict: None,
        },
        Ok(Some(conflict)) => {
            let mut detail = restore_one_checkpoint(ckpt, target);
            detail.forced_conflict = Some(conflict);
            detail
        }
        Ok(None) => restore_one_checkpoint(ckpt, target),
        Err(err) => RestoreDetail {
            kind: RestoreKind::Failed,
            label: "failed",
            message: err.to_string(),
            conflict: None,
            forced_conflict: None,
        },
    }
}

fn detect_reset_conflict(
    expected_after_hash: Option<&str>,
    target: &Path,
) -> Result<Option<ResetConflict>, CheckpointError> {
    let Some(expected_after_hash) = expected_after_hash else {
        return Ok(Some(ResetConflict {
            expected_after_hash: "__missing_after_hash__".to_string(),
            current_hash: current_file_fingerprint(target)?,
            reason: "missing_after_hash",
        }));
    };
    let current_hash = current_file_fingerprint(target)?;
    if current_hash == expected_after_hash {
        return Ok(None);
    }
    Ok(Some(ResetConflict {
        expected_after_hash: expected_after_hash.to_string(),
        current_hash,
        reason: "after_hash_mismatch",
    }))
}

fn restore_one_checkpoint(ckpt: &FileCheckpointRecord, target: &Path) -> RestoreDetail {
    let was_missing = ckpt
        .before_hash
        .as_deref()
        .map(|h| h == MISSING_FILE_SENTINEL)
        .unwrap_or(false)
        || ckpt.before_size < 0;

    if was_missing {
        if !target.exists() {
            return RestoreDetail {
                kind: RestoreKind::Deleted,
                label: "deleted",
                message: "file already absent".to_string(),
                conflict: None,
                forced_conflict: None,
            };
        }
        match std::fs::remove_file(target) {
            Ok(_) => RestoreDetail {
                kind: RestoreKind::Deleted,
                label: "deleted",
                message: "removed file created by task".to_string(),
                conflict: None,
                forced_conflict: None,
            },
            Err(err) => RestoreDetail {
                kind: RestoreKind::Failed,
                label: "failed",
                message: format!("remove failed: {err}"),
                conflict: None,
                forced_conflict: None,
            },
        }
    } else if let Some(content) = ckpt.before_content.as_deref() {
        write_with_parent_dirs(target, content.as_bytes())
    } else if let Some(blob_path) = ckpt.before_blob_path.as_deref() {
        match std::fs::read(blob_path) {
            Ok(bytes) => write_with_parent_dirs(target, &bytes),
            Err(err) => RestoreDetail {
                kind: RestoreKind::Failed,
                label: "failed",
                message: format!("read blob failed: {err}"),
                conflict: None,
                forced_conflict: None,
            },
        }
    } else {
        RestoreDetail {
            kind: RestoreKind::Failed,
            label: "failed",
            message: "checkpoint has neither inline content nor blob path".to_string(),
            conflict: None,
            forced_conflict: None,
        }
    }
}

fn write_with_parent_dirs(target: &Path, bytes: &[u8]) -> RestoreDetail {
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                return RestoreDetail {
                    kind: RestoreKind::Failed,
                    label: "failed",
                    message: format!("create parent failed: {err}"),
                    conflict: None,
                    forced_conflict: None,
                };
            }
        }
    }
    match std::fs::write(target, bytes) {
        Ok(_) => RestoreDetail {
            kind: RestoreKind::Restored,
            label: "restored",
            message: format!("wrote {} bytes", bytes.len()),
            conflict: None,
            forced_conflict: None,
        },
        Err(err) => RestoreDetail {
            kind: RestoreKind::Failed,
            label: "failed",
            message: format!("write failed: {err}"),
            conflict: None,
            forced_conflict: None,
        },
    }
}

// ---------------------------------------------------------------------------
// purge_run_checkpoints tool
// ---------------------------------------------------------------------------

pub struct PurgeRunCheckpointsTool {
    db: LocalDb,
    current_session_id: Option<String>,
}

impl PurgeRunCheckpointsTool {
    pub fn new(db: LocalDb, current_session_id: Option<String>) -> Self {
        Self {
            db,
            current_session_id,
        }
    }
}

#[async_trait]
impl Tool for PurgeRunCheckpointsTool {
    fn name(&self) -> &str {
        "purge_run_checkpoints"
    }

    fn description(&self) -> &str {
        "Delete file checkpoint blobs and DB rows for a completed run to reclaim disk space."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Purge run_file_checkpoints rows (and their on-disk blobs) for the given \
                run_id, or for the current session if run_id is omitted. Use only after the run is \
                truly complete; you lose the ability to reset_task afterwards."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "run_id": {
                        "type": "string",
                        "description": "Run id whose checkpoints should be purged. If omitted, purge all checkpoints for the current session's active task."
                    }
                }
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "清理 run checkpoint".to_string(),
            description_zh: "清掉一个 run 已落地的 checkpoint blob + 数据库记录。".to_string(),
            capability_labels_zh: vec!["文件系统".to_string(), "清理".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::Filesystem, ToolCapability::System],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let run_id = args
            .get("run_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let outcome = match run_id.as_deref() {
            Some(rid) => purge_for_run(&self.db, rid).map_err(|e| e.into_agent_error())?,
            None => {
                let sid = self.current_session_id.as_ref().ok_or_else(|| {
                    AgentError::Tool(
                        "当前没有绑定会话，也未指定 run_id，无法清理 checkpoint。".to_string(),
                    )
                })?;
                let active = self
                    .db
                    .get_active_plan_task(sid)
                    .map_err(|e| AgentError::Tool(e.to_string()))?
                    .ok_or_else(|| {
                        AgentError::Tool(
                            "当前没有激活任务，无法在未指定 run_id 时清理 checkpoint。".to_string(),
                        )
                    })?;
                purge_for_task(&self.db, &active.id).map_err(|e| e.into_agent_error())?
            }
        };

        Ok(ToolResult::success(
            format!(
                "已清理 {} 条 checkpoint（删除 blob {} 个，失败 {}）。",
                outcome.deleted_rows, outcome.deleted_blobs, outcome.failures
            ),
            json!({
                "deletedRows": outcome.deleted_rows,
                "deletedBlobs": outcome.deleted_blobs,
                "failures": outcome.failures,
            }),
        ))
    }
}

#[derive(Debug, Default)]
pub struct PurgeOutcome {
    pub deleted_rows: usize,
    pub deleted_blobs: usize,
    pub failures: usize,
}

pub fn purge_for_run(db: &LocalDb, run_id: &str) -> Result<PurgeOutcome, CheckpointError> {
    let records = db
        .list_file_checkpoints_by_run(run_id)
        .map_err(|e| CheckpointError::Db(e.to_string()))?;
    purge_records(db, &records)
}

pub fn purge_for_task(db: &LocalDb, task_id: &str) -> Result<PurgeOutcome, CheckpointError> {
    let records = db
        .list_file_checkpoints(task_id)
        .map_err(|e| CheckpointError::Db(e.to_string()))?;
    purge_records(db, &records)
}

fn purge_records(
    db: &LocalDb,
    records: &[FileCheckpointRecord],
) -> Result<PurgeOutcome, CheckpointError> {
    let mut outcome = PurgeOutcome::default();
    for ckpt in records {
        if let Some(blob) = ckpt.before_blob_path.as_deref() {
            match std::fs::remove_file(blob) {
                Ok(_) => outcome.deleted_blobs += 1,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    // already gone; not a failure
                }
                Err(_) => outcome.failures += 1,
            }
        }
        match db.delete_file_checkpoint(&ckpt.id) {
            Ok(_) => outcome.deleted_rows += 1,
            Err(_) => outcome.failures += 1,
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        let path = std::env::temp_dir().join(format!("aura_ckpt_test_{}.db", Uuid::new_v4()));
        LocalDb::open(path).unwrap()
    }

    #[test]
    fn file_content_writers_honor_checkpoint_contract() {
        // M-9 (d): a tripwire so a NEW filesystem-mutating tool cannot slip into
        // the registry without a conscious checkpoint-contract decision. If you
        // register a new fs-mutating tool this assertion fails; update the
        // expected set, and if the tool overwrites existing file content also add
        // it to FILE_CONTENT_WRITE_TOOLS and call capture_before_write +
        // record_after_write in its execute().
        let registry = crate::create_tool_registry(temp_db());
        let mut fs_mutating: Vec<String> = registry
            .list_metadata()
            .into_iter()
            .filter(|m| m.capabilities.contains(&ToolCapability::Filesystem) && m.mutates_state)
            .map(|m| m.name)
            .collect();
        fs_mutating.sort();

        let mut expected = vec![
            "create_directory",
            "edit_file",
            "git_commit",
            "git_create_branch",
            "git_push",
            "git_stage",
            "purge_run_checkpoints",
            "reset_task",
            "write_file",
        ];
        expected.sort();
        assert_eq!(
            fs_mutating, expected,
            "filesystem-mutating tool set changed; review checkpoint-contract \
             coverage (FILE_CONTENT_WRITE_TOOLS) before updating this list"
        );

        // Every declared file-content writer must actually be a registered
        // fs-mutating tool — no stale or misspelled contract entries.
        for name in FILE_CONTENT_WRITE_TOOLS {
            assert!(
                fs_mutating.iter().any(|registered| registered == name),
                "checkpoint-contract tool {name} is not a registered fs-mutating tool"
            );
            assert!(honors_file_checkpoint_contract(name));
        }
        assert!(!honors_file_checkpoint_contract("create_directory"));
    }

    fn setup_session_with_active_task(db: &LocalDb) -> (String, String) {
        let session = db.create_session("ckpt-test").unwrap();
        let task = db
            .create_plan_task_full(&session.id, "do work", None, None, "test", None, None)
            .unwrap();
        db.set_active_plan_task(&session.id, Some(&task.id))
            .unwrap();
        (session.id, task.id)
    }

    fn setup_session_with_active_run_task(db: &LocalDb) -> (String, String, String) {
        let session = db.create_session("ckpt-run-diff-test").unwrap();
        let run_id = format!("run-{}", Uuid::new_v4());
        db.create_agent_run(&run_id, Some(&session.id), "default")
            .unwrap();
        let task = db
            .create_plan_task_full(
                &session.id,
                "do work",
                None,
                Some(&run_id),
                "test",
                None,
                None,
            )
            .unwrap();
        db.set_active_plan_task(&session.id, Some(&task.id))
            .unwrap();
        (session.id, task.id, run_id)
    }

    #[test]
    fn capture_inline_for_small_existing_file() {
        let db = temp_db();
        let (sid, _tid) = setup_session_with_active_task(&db);
        let target = std::env::temp_dir().join(format!("aura_ckpt_small_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, b"hello").unwrap();

        let outcome = capture_before_write(&db, Some(&sid), &target).unwrap();
        match outcome {
            CheckpointOutcome::Captured(record) => {
                assert_eq!(record.before_content.as_deref(), Some("hello"));
                assert_eq!(
                    record.before_hash.as_deref(),
                    Some("sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
                );
                assert!(record.before_blob_path.is_none());
                assert_eq!(record.before_size, 5);
            }
            other => panic!("expected captured, got {other:?}"),
        }
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn capture_missing_marker_when_file_does_not_exist() {
        let db = temp_db();
        let (sid, _tid) = setup_session_with_active_task(&db);
        let target = std::env::temp_dir().join(format!("aura_ckpt_missing_{}.txt", Uuid::new_v4()));
        // ensure non-existent
        let _ = std::fs::remove_file(&target);

        let outcome = capture_before_write(&db, Some(&sid), &target).unwrap();
        match outcome {
            CheckpointOutcome::Captured(record) => {
                assert_eq!(record.before_hash.as_deref(), Some(MISSING_FILE_SENTINEL));
                assert!(record.before_content.is_none());
                assert!(record.before_blob_path.is_none());
                assert_eq!(record.before_size, -1);
            }
            other => panic!("expected captured, got {other:?}"),
        }
    }

    #[test]
    fn capture_rejects_over_hard_limit() {
        let db = temp_db();
        let (sid, _tid) = setup_session_with_active_task(&db);
        let target = std::env::temp_dir().join(format!("aura_ckpt_huge_{}.bin", Uuid::new_v4()));
        // 6 MB > HARD_LIMIT_BYTES
        let big = vec![0u8; (HARD_LIMIT_BYTES + 1024) as usize];
        std::fs::write(&target, &big).unwrap();

        let err = capture_before_write(&db, Some(&sid), &target).unwrap_err();
        assert!(matches!(err, CheckpointError::FileTooLarge { .. }));
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn capture_skips_when_no_active_task() {
        let db = temp_db();
        let session = db.create_session("ckpt-noactive").unwrap();
        let target = std::env::temp_dir().join(format!("aura_ckpt_noact_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, b"x").unwrap();
        let outcome = capture_before_write(&db, Some(&session.id), &target).unwrap();
        assert!(matches!(
            outcome,
            CheckpointOutcome::Skipped(CheckpointSkipReason::NoActiveTask)
        ));
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn reset_task_restores_inline_and_deletes_missing() {
        let db = temp_db();
        let (sid, tid) = setup_session_with_active_task(&db);

        // Existing file: capture, then mutate, then reset should restore.
        let kept =
            std::env::temp_dir().join(format!("aura_ckpt_reset_kept_{}.txt", Uuid::new_v4()));
        std::fs::write(&kept, b"original").unwrap();
        let kept_checkpoint = capture_before_write(&db, Some(&sid), &kept).unwrap();
        std::fs::write(&kept, b"mutated").unwrap();
        record_after_write(&db, &kept_checkpoint, &kept).unwrap();

        // New file: capture before (does not exist), then create, reset should delete.
        let new_file =
            std::env::temp_dir().join(format!("aura_ckpt_reset_new_{}.txt", Uuid::new_v4()));
        let _ = std::fs::remove_file(&new_file);
        let new_checkpoint = capture_before_write(&db, Some(&sid), &new_file).unwrap();
        std::fs::write(&new_file, b"created-by-task").unwrap();
        record_after_write(&db, &new_checkpoint, &new_file).unwrap();

        let outcome = perform_reset_task(&db, &tid).unwrap();
        assert_eq!(outcome.total, 2);
        assert_eq!(outcome.restored, 1);
        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.failed, 0);
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "original");
        assert!(!new_file.exists());
        let _ = std::fs::remove_file(kept);
    }

    #[test]
    fn record_after_write_stores_text_snapshot_for_run_diff() {
        let db = temp_db();
        let (sid, tid) = setup_session_with_active_task(&db);
        let target =
            std::env::temp_dir().join(format!("aura_ckpt_after_content_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, b"before\n").unwrap();

        let checkpoint = capture_before_write(&db, Some(&sid), &target).unwrap();
        std::fs::write(&target, b"after\n").unwrap();
        record_after_write(&db, &checkpoint, &target).unwrap();

        let saved = db.list_file_checkpoints(&tid).unwrap();
        assert_eq!(saved[0].after_content.as_deref(), Some("after\n"));
        assert!(saved[0]
            .after_hash
            .as_deref()
            .is_some_and(|hash| hash.starts_with("sha256:")));
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn run_diff_uses_checkpoint_snapshots_not_current_file() {
        let db = temp_db();
        let (sid, _tid, run_id) = setup_session_with_active_run_task(&db);
        let target = std::env::temp_dir().join(format!("aura_ckpt_diff_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, "one\ntwo\n").unwrap();

        let checkpoint = capture_before_write(&db, Some(&sid), &target).unwrap();
        std::fs::write(&target, "one\nthree\nfour\n").unwrap();
        record_after_write(&db, &checkpoint, &target).unwrap();

        // Later disk changes must not rewrite the historical run diff.
        std::fs::write(&target, "later user edit\n").unwrap();
        let diff = build_run_diff(&db, &run_id, 20).unwrap();

        assert_eq!(diff.total_files, 1);
        assert_eq!(diff.returned_files, 1);
        let file = &diff.files[0];
        assert_eq!(file.status, "modified");
        assert_eq!(file.additions, 2);
        assert_eq!(file.deletions, 1);
        assert!(file.stats_accurate);
        let text = file.diff_text.as_deref().unwrap();
        assert!(text.contains("-two"));
        assert!(text.contains("+three"));
        assert!(text.contains("+four"));
        assert!(!text.contains("later user edit"));
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn run_diff_marks_old_checkpoints_without_after_snapshot_unavailable() {
        let db = temp_db();
        let (sid, _tid, run_id) = setup_session_with_active_run_task(&db);
        let target =
            std::env::temp_dir().join(format!("aura_ckpt_diff_legacy_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, "before\n").unwrap();

        let _checkpoint = capture_before_write(&db, Some(&sid), &target).unwrap();
        std::fs::write(&target, "after without snapshot\n").unwrap();
        let diff = build_run_diff(&db, &run_id, 20).unwrap();

        assert_eq!(diff.total_files, 1);
        let file = &diff.files[0];
        assert_eq!(
            file.unavailable_reason.as_deref(),
            Some("after_content_missing")
        );
        assert!(file.diff_text.is_none());
        assert!(!file.stats_accurate);
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn reset_task_detects_user_edit_conflict_without_overwriting() {
        let db = temp_db();
        let (sid, tid) = setup_session_with_active_task(&db);
        let target =
            std::env::temp_dir().join(format!("aura_ckpt_conflict_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, b"original").unwrap();

        let checkpoint = capture_before_write(&db, Some(&sid), &target).unwrap();
        std::fs::write(&target, b"task edit").unwrap();
        record_after_write(&db, &checkpoint, &target).unwrap();

        std::fs::write(&target, b"user edit").unwrap();
        let outcome = perform_reset_task(&db, &tid).unwrap();

        assert_eq!(outcome.total, 1);
        assert_eq!(outcome.conflicts, 1);
        assert_eq!(outcome.restored, 0);
        assert_eq!(outcome.failed, 0);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "user edit");
        assert!(
            db.list_file_checkpoints(&tid).unwrap()[0]
                .restored_at
                .is_none(),
            "conflicted checkpoints must remain un-restored for later force/reset"
        );
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn reset_task_uses_latest_after_hash_for_repeated_writes_to_same_file() {
        let db = temp_db();
        let (sid, tid) = setup_session_with_active_task(&db);
        let target =
            std::env::temp_dir().join(format!("aura_ckpt_repeated_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, b"original").unwrap();

        let first = capture_before_write(&db, Some(&sid), &target).unwrap();
        std::fs::write(&target, b"task edit 1").unwrap();
        record_after_write(&db, &first, &target).unwrap();

        let second = capture_before_write(&db, Some(&sid), &target).unwrap();
        std::fs::write(&target, b"task edit 2").unwrap();
        record_after_write(&db, &second, &target).unwrap();

        let outcome = perform_reset_task(&db, &tid).unwrap();

        assert_eq!(outcome.total, 1);
        assert_eq!(outcome.conflicts, 0);
        assert_eq!(outcome.restored, 1);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        assert!(
            db.list_file_checkpoints(&tid)
                .unwrap()
                .iter()
                .all(|checkpoint| checkpoint.restored_at.is_some()),
            "all checkpoints for a restored path should be marked restored"
        );
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn reset_task_treats_missing_after_hash_as_conflict_until_forced() {
        let db = temp_db();
        let (sid, tid) = setup_session_with_active_task(&db);
        let target = std::env::temp_dir().join(format!("aura_ckpt_legacy_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, b"original").unwrap();

        let _checkpoint = capture_before_write(&db, Some(&sid), &target).unwrap();
        std::fs::write(&target, b"task edit without after hash").unwrap();

        let blocked = perform_reset_task(&db, &tid).unwrap();
        assert_eq!(blocked.conflicts, 1);
        assert_eq!(blocked.restored, 0);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "task edit without after hash"
        );

        let forced =
            perform_reset_task_with_options(&db, &tid, ResetTaskOptions { force: true }).unwrap();
        assert_eq!(forced.forced_conflicts, 1);
        assert_eq!(forced.restored, 1);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn reset_task_force_overwrites_conflict_when_requested() {
        let db = temp_db();
        let (sid, tid) = setup_session_with_active_task(&db);
        let target = std::env::temp_dir().join(format!("aura_ckpt_force_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, b"original").unwrap();

        let checkpoint = capture_before_write(&db, Some(&sid), &target).unwrap();
        std::fs::write(&target, b"task edit").unwrap();
        record_after_write(&db, &checkpoint, &target).unwrap();

        std::fs::write(&target, b"user edit").unwrap();
        let outcome =
            perform_reset_task_with_options(&db, &tid, ResetTaskOptions { force: true }).unwrap();

        assert_eq!(outcome.total, 1);
        assert_eq!(outcome.conflicts, 0);
        assert_eq!(outcome.forced_conflicts, 1);
        assert_eq!(outcome.restored, 1);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        assert!(
            db.list_file_checkpoints(&tid).unwrap()[0]
                .restored_at
                .is_some(),
            "forced rollback should mark the checkpoint restored"
        );
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn reset_task_detects_created_file_conflict_without_deleting() {
        let db = temp_db();
        let (sid, tid) = setup_session_with_active_task(&db);
        let target =
            std::env::temp_dir().join(format!("aura_ckpt_created_conflict_{}.txt", Uuid::new_v4()));
        let _ = std::fs::remove_file(&target);

        let checkpoint = capture_before_write(&db, Some(&sid), &target).unwrap();
        std::fs::write(&target, b"created by task").unwrap();
        record_after_write(&db, &checkpoint, &target).unwrap();

        std::fs::write(&target, b"user changed created file").unwrap();
        let outcome = perform_reset_task(&db, &tid).unwrap();

        assert_eq!(outcome.total, 1);
        assert_eq!(outcome.conflicts, 1);
        assert_eq!(outcome.deleted, 0);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "user changed created file"
        );
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn capture_external_blob_for_oversize_file() {
        // File > INLINE_LIMIT but ≤ HARD_LIMIT must go to external blob storage.
        let db = temp_db();
        let (sid, tid) = setup_session_with_active_task(&db);
        let target = std::env::temp_dir().join(format!("aura_ckpt_blob_{}.bin", Uuid::new_v4()));
        let payload = vec![0xAAu8; (INLINE_LIMIT_BYTES + 1024) as usize];
        std::fs::write(&target, &payload).unwrap();

        let outcome = capture_before_write(&db, Some(&sid), &target).unwrap();
        let record = match outcome {
            CheckpointOutcome::Captured(r) => r,
            other => panic!("expected captured, got {other:?}"),
        };
        assert!(
            record.before_content.is_none(),
            "must not inline large files"
        );
        let blob_path = record
            .before_blob_path
            .as_deref()
            .expect("external blob path expected");
        assert!(
            std::path::Path::new(blob_path).exists(),
            "blob file must exist on disk: {blob_path}"
        );
        assert_eq!(record.before_size, payload.len() as i64);

        // Round-trip: mutate target, reset, ensure blob restored byte-for-byte.
        std::fs::write(&target, b"clobbered").unwrap();
        record_after_write(&db, &CheckpointOutcome::Captured(record), &target).unwrap();
        let outcome = perform_reset_task(&db, &tid).unwrap();
        assert_eq!(outcome.restored, 1);
        let restored = std::fs::read(&target).unwrap();
        assert_eq!(restored.len(), payload.len());
        assert_eq!(&restored[..16], &payload[..16]);
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn purge_for_task_removes_rows_and_blobs() {
        let db = temp_db();
        let (sid, tid) = setup_session_with_active_task(&db);
        let target = std::env::temp_dir().join(format!("aura_ckpt_purge_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, b"to-be-purged").unwrap();
        let _ = capture_before_write(&db, Some(&sid), &target).unwrap();
        assert_eq!(db.list_file_checkpoints(&tid).unwrap().len(), 1);

        let outcome = purge_for_task(&db, &tid).unwrap();
        assert_eq!(outcome.deleted_rows, 1);
        assert_eq!(outcome.failures, 0);
        assert!(db.list_file_checkpoints(&tid).unwrap().is_empty());
        let _ = std::fs::remove_file(target);
    }
}
