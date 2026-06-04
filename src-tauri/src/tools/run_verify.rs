//! `run_verify` tool — execute a verify command for the active plan task,
//! record the outcome in `run_task_verifications`, and enforce plan §M3.4's
//! "same stderr signature appears 3 times → mark task blocked" rule.
//!
//! Resolution order for the command (mirrors `agent::verification::load_verify_config`):
//! 1. explicit `command` argument
//! 2. `kind` argument resolved via task `verify_json`, then project `.aura/verify.toml`,
//!    then `~/.aura/verify.toml`, then builtin defaults.
//! 3. error if neither resolves.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::agent::verification::{load_verify_config, stderr_signature, user_aura_home};
use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::storage::LocalDb;
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};

const VERIFY_TIMEOUT_SECS: u64 = 300;
const TAIL_CHARS: usize = 4_000;
const SAME_SIGNATURE_BLOCK_THRESHOLD: usize = 3;

pub struct RunVerifyTool {
    db: LocalDb,
    project_root: Option<PathBuf>,
    current_session_id: Option<String>,
}

impl RunVerifyTool {
    pub fn new(
        db: LocalDb,
        project_root: Option<PathBuf>,
        current_session_id: Option<String>,
    ) -> Self {
        Self {
            db,
            project_root,
            current_session_id,
        }
    }
}

#[async_trait]
impl Tool for RunVerifyTool {
    fn name(&self) -> &str {
        "run_verify"
    }

    fn description(&self) -> &str {
        "Run a verify command for the active plan task and record the outcome."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Execute a verify command (resolved by `kind` or passed via `command`) \
                in the project working directory, then record the result in run_task_verifications. \
                When the same stderr signature fails 3 times in a row for the active task, the task \
                status is automatically flipped to `blocked` (evidence_status=failed)."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "description": "Verification kind to resolve via task verify / project / user / builtin tables (e.g. rust_check, frontend_build, full_verify)."
                    },
                    "command": {
                        "type": "string",
                        "description": "Explicit shell command. If both kind and command are given, command wins."
                    },
                    "task_id": {
                        "type": "string",
                        "description": "Optional plan task id; defaults to the session's active task."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory; defaults to the registered project root or process cwd."
                    }
                }
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "运行验证命令".to_string(),
            description_zh: "为当前活跃任务跑一条 verify 命令并记录结果。".to_string(),
            capability_labels_zh: vec!["系统命令".to_string(), "验证".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::System],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let session_id = self
            .current_session_id
            .as_ref()
            .ok_or_else(|| AgentError::Tool("当前没有绑定会话，无法跑 verify。".to_string()))?
            .clone();

        let task_id_arg = args
            .get("task_id")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let task = match task_id_arg {
            Some(id) => self
                .db
                .get_plan_task(&id)
                .map_err(|e| AgentError::Tool(format!("找不到 task_id={id}：{e}")))?,
            None => self
                .db
                .get_active_plan_task(&session_id)
                .map_err(|e| AgentError::Tool(e.to_string()))?
                .ok_or_else(|| {
                    AgentError::Tool(
                        "当前没有激活任务；先 set_active_plan_task 或传 task_id。".to_string(),
                    )
                })?,
        };

        let kind = args
            .get("kind")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string());
        let explicit_command = args
            .get("command")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let command = match (explicit_command.clone(), kind.clone()) {
            (Some(cmd), _) => cmd,
            (None, Some(k)) if !k.is_empty() => {
                let cfg = load_verify_config(
                    self.project_root.as_deref(),
                    user_aura_home().as_deref(),
                    Some(&task.verify),
                );
                cfg.commands.get(&k).cloned().ok_or_else(|| {
                    AgentError::Tool(format!(
                        "找不到 verify kind=`{k}`。可用：{}",
                        cfg.commands.keys().cloned().collect::<Vec<_>>().join(", ")
                    ))
                })?
            }
            _ => {
                return Err(AgentError::Tool(
                    "请提供 kind 或 command 之一。".to_string(),
                ))
            }
        };

        let resolved_kind = kind.unwrap_or_else(|| "manual".to_string());
        let cwd_arg = args.get("cwd").and_then(Value::as_str).map(PathBuf::from);
        let cwd = cwd_arg
            .or_else(|| self.project_root.clone())
            .or_else(|| std::env::current_dir().ok());

        // T5: even though we go through run_verify, refuse outright destructive commands.
        if let crate::tools::command_safety::CommandSafety::Denied { reason } =
            crate::tools::command_safety::classify_command(&command)
        {
            return Err(AgentError::Tool(format!(
                "verify 命令被命令安全网关拒绝：{reason}"
            )));
        }

        let started_at = now_ms();
        let (stdout, stderr, exit_code_raw, timed_out) =
            execute_command(&command, cwd.as_deref()).await;
        let finished_at = now_ms();
        let exit_code: Option<i64> = exit_code_raw.map(|c| c as i64);
        let status_label = if timed_out {
            "timeout"
        } else if exit_code_raw == Some(0) {
            "passed"
        } else {
            "failed"
        };

        let record = self
            .db
            .record_task_verification(
                &task.id,
                task.run_id.as_deref(),
                &resolved_kind,
                &command,
                exit_code,
                status_label,
                &tail(&stdout),
                &tail(&stderr),
                started_at,
                Some(finished_at),
            )
            .map_err(|e| AgentError::Tool(e.to_string()))?;

        let signature = stderr_signature(&stderr);
        let mut auto_blocked = false;
        let mut auto_block_reason: Option<String> = None;

        if status_label == "passed" {
            // T7.1 — successful verification flips evidence_status to verified
            // when the task is already done, otherwise leaves status alone.
            let _ = self
                .db
                .update_plan_task_evidence(
                    &task.id,
                    Some(&json!({
                        "kind": resolved_kind,
                        "verification_id": record.id,
                        "command": command,
                        "exit_code": exit_code,
                    })),
                    "verified",
                    None,
                )
                .map_err(|e| AgentError::Tool(e.to_string()))?;
        } else if status_label == "failed" || status_label == "timeout" {
            let history = self
                .db
                .list_task_verifications(&task.id)
                .map_err(|e| AgentError::Tool(e.to_string()))?;
            let recent_same: usize = history
                .iter()
                .take(SAME_SIGNATURE_BLOCK_THRESHOLD + 2)
                .filter(|v| {
                    matches!(v.status.as_str(), "failed" | "timeout")
                        && stderr_signature(&v.stderr_tail) == signature
                })
                .count();
            if recent_same >= SAME_SIGNATURE_BLOCK_THRESHOLD {
                let reason = format!(
                    "verify `{resolved_kind}` 连续 {recent_same} 次同样的 stderr 签名失败，已自动 blocked。"
                );
                let _ = self.db.update_plan_task_status(&task.id, "blocked", None);
                let _ = self.db.update_plan_task_evidence(
                    &task.id,
                    Some(&json!({
                        "auto_blocked": true,
                        "kind": resolved_kind,
                        "signature": signature,
                        "verification_id": record.id,
                    })),
                    "failed",
                    Some(&reason),
                );
                auto_blocked = true;
                auto_block_reason = Some(reason);
            }
        }

        let summary = match (status_label, auto_blocked) {
            ("passed", _) => format!("verify {resolved_kind} 通过。"),
            (_, true) => "verify 连续失败，任务已自动 blocked。".to_string(),
            ("timeout", _) => format!("verify {resolved_kind} 超时。"),
            (_, _) => format!("verify {resolved_kind} 失败，请修复后再跑。"),
        };

        Ok(ToolResult::success(
            summary,
            json!({
                "taskId": task.id,
                "kind": resolved_kind,
                "command": command,
                "exitCode": exit_code,
                "status": status_label,
                "signature": signature,
                "stderrTail": tail(&stderr),
                "stdoutTail": tail(&stdout),
                "verificationId": record.id,
                "autoBlocked": auto_blocked,
                "autoBlockReason": auto_block_reason,
                "timedOut": timed_out,
            }),
        ))
    }
}

/// T24: outcome of an auto-run verify hook.
pub struct AutoVerifyOutcome {
    pub passed: bool,
    /// Set when the same-signature repair loop kicked in (T25).
    pub blocked: bool,
    /// Reason text already written to plan_tasks.blocked_reason. Callers must
    /// preserve this when issuing further plan_task updates.
    pub blocked_reason: Option<String>,
}

/// P2-2: a single resolved verify entry from `task.verify[]`.
struct VerifyEntry {
    command: String,
    /// Defaults to true. An optional entry (`required:false`) that fails does
    /// NOT block the `done` transition.
    required: bool,
    /// Verify kind recorded as the evidence source; defaults to `auto_done`.
    kind: String,
}

/// P2-2: resolve **every** runnable entry from `task.verify`, preserving order.
/// Accepts `Array<String>` and `Array<{command, required?, kind?}>`. Blank or
/// missing commands are skipped. `required` defaults to true; `kind` defaults
/// to `auto_done`.
fn all_verify_entries(verify: &Value) -> Vec<VerifyEntry> {
    let Some(arr) = verify.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in arr {
        match entry {
            Value::String(s) if !s.trim().is_empty() => out.push(VerifyEntry {
                command: s.trim().to_string(),
                required: true,
                kind: "auto_done".to_string(),
            }),
            Value::Object(map) => {
                if let Some(s) = map.get("command").and_then(Value::as_str) {
                    if !s.trim().is_empty() {
                        out.push(VerifyEntry {
                            command: s.trim().to_string(),
                            required: map.get("required").and_then(Value::as_bool).unwrap_or(true),
                            kind: map
                                .get("kind")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|k| !k.is_empty())
                                .unwrap_or("auto_done")
                                .to_string(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// T24 + P2-2: auto-run **every** verify entry for a task that just
/// transitioned to `done` (called from UpdatePlanTaskTool). Iterates
/// `task.verify` (Array<String> or Array<{command, required?, kind?}>),
/// executes each runnable entry, and records its own row in
/// `run_task_verifications`.
///
/// Aggregation (P2-2): the task passes only when **every required** entry
/// passes. An optional entry (`required:false`) may fail without blocking
/// `done`. Denied (destructive) commands are skipped. Returns `None` only when
/// no runnable entry exists.
///
/// On all-required-pass → evidence_status=verified; on any required failure →
/// evidence_status=failed. The same-signature repair loop (T25) still flips the
/// task to `blocked` when a *required* entry fails with an identical stderr
/// signature `SAME_SIGNATURE_BLOCK_THRESHOLD` times.
pub async fn auto_verify_done_task(
    db: &LocalDb,
    task: &crate::storage::PlanTaskRecord,
    project_root: Option<&std::path::Path>,
) -> Option<AutoVerifyOutcome> {
    let entries = all_verify_entries(&task.verify);
    if entries.is_empty() {
        return None;
    }
    let cwd = project_root
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());

    let mut ran_any = false;
    let mut all_required_passed = true;
    let mut blocked = false;
    let mut blocked_reason: Option<String> = None;
    let mut runs: Vec<Value> = Vec::new();

    for entry in &entries {
        if let crate::tools::command_safety::CommandSafety::Denied { reason } =
            crate::tools::command_safety::classify_command(&entry.command)
        {
            let now = now_ms();
            let status_label = if entry.required { "failed" } else { "skipped" };
            let stderr = format!("verify 命令被命令安全网关拒绝：{reason}");
            let Some(record) = db
                .record_task_verification(
                    &task.id,
                    task.run_id.as_deref(),
                    &entry.kind,
                    &entry.command,
                    None,
                    status_label,
                    "",
                    &stderr,
                    now,
                    Some(now),
                )
                .ok()
            else {
                continue;
            };
            ran_any = true;
            runs.push(json!({
                "verification_id": record.id,
                "kind": entry.kind,
                "command": entry.command,
                "exit_code": Value::Null,
                "status": status_label,
                "required": entry.required,
                "denied": true,
            }));
            if entry.required {
                all_required_passed = false;
            }
            continue;
        }
        let started_at = now_ms();
        let (stdout, stderr, exit_code_raw, timed_out) =
            execute_command(&entry.command, cwd.as_deref()).await;
        let finished_at = now_ms();
        let exit_code: Option<i64> = exit_code_raw.map(|c| c as i64);
        let status_label = if timed_out {
            "timeout"
        } else if exit_code_raw == Some(0) {
            "passed"
        } else {
            "failed"
        };
        let Some(record) = db
            .record_task_verification(
                &task.id,
                task.run_id.as_deref(),
                &entry.kind,
                &entry.command,
                exit_code,
                status_label,
                &tail(&stdout),
                &tail(&stderr),
                started_at,
                Some(finished_at),
            )
            .ok()
        else {
            continue;
        };
        ran_any = true;
        runs.push(json!({
            "verification_id": record.id,
            "kind": entry.kind,
            "command": entry.command,
            "exit_code": exit_code,
            "status": status_label,
            "required": entry.required,
        }));

        if status_label == "passed" {
            continue;
        }
        if entry.required {
            all_required_passed = false;
        }
        // T25: only a *required* failure can trip the same-signature repair loop.
        if entry.required && !blocked {
            let signature = stderr_signature(&stderr);
            if let Ok(history) = db.list_task_verifications(&task.id) {
                let recent_same = history
                    .iter()
                    .take(SAME_SIGNATURE_BLOCK_THRESHOLD + 2)
                    .filter(|v| {
                        matches!(v.status.as_str(), "failed" | "timeout")
                            && stderr_signature(&v.stderr_tail) == signature
                    })
                    .count();
                if recent_same >= SAME_SIGNATURE_BLOCK_THRESHOLD {
                    blocked = true;
                    blocked_reason = Some(format!(
                        "auto-verify `{}` 连续 {recent_same} 次同样的 stderr 签名失败，已自动 blocked。",
                        entry.kind
                    ));
                }
            }
        }
    }

    if !ran_any {
        return None;
    }

    if blocked {
        let _ = db.update_plan_task_status(&task.id, "blocked", None);
        let _ = db.update_plan_task_evidence(
            &task.id,
            Some(&json!({ "auto_blocked": true, "auto_verify": true, "runs": runs })),
            "failed",
            blocked_reason.as_deref(),
        );
        return Some(AutoVerifyOutcome {
            passed: false,
            blocked: true,
            blocked_reason,
        });
    }

    let (evidence_status, passed) = if all_required_passed {
        ("verified", true)
    } else {
        ("failed", false)
    };
    let _ = db.update_plan_task_evidence(
        &task.id,
        Some(&json!({
            "auto_verify": true,
            "all_required_passed": all_required_passed,
            "runs": runs,
        })),
        evidence_status,
        None,
    );
    Some(AutoVerifyOutcome {
        passed,
        blocked: false,
        blocked_reason: None,
    })
}

/// P2-1/P2-2: result of one verify entry that ran after a `run_command`.
/// Surfaced back to the model so a failing build/test gets repaired mid-run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoVerifyReport {
    pub command: String,
    pub passed: bool,
    pub exit_code: Option<i64>,
    pub stderr_tail: String,
    /// P2-2: false for entries marked `required:false`. A failing optional entry
    /// is still reported but flagged so the model treats it as non-blocking.
    pub required: bool,
}

/// P2-1 + P2-2: run **every** verify entry of the active task after a matching
/// `run_command`, recording each as evidence (`run_task_verifications`, source
/// `auto_command`). Unlike the done-time auto-verify, this does NOT flip task
/// status — it is a mid-run safety net whose per-entry verdicts are fed back to
/// the model. Returns `None` when the task has no runnable verify entry. The
/// matcher gate (whether to call this at all) lives in the caller, which loads
/// the verify config.
pub async fn verify_after_command(
    db: &LocalDb,
    task: &crate::storage::PlanTaskRecord,
    project_root: Option<&std::path::Path>,
) -> Option<Vec<AutoVerifyReport>> {
    let entries = all_verify_entries(&task.verify);
    if entries.is_empty() {
        return None;
    }
    let cwd = project_root
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());

    let mut reports = Vec::new();
    for entry in &entries {
        if let crate::tools::command_safety::CommandSafety::Denied { reason } =
            crate::tools::command_safety::classify_command(&entry.command)
        {
            let now = now_ms();
            let status_label = if entry.required { "failed" } else { "skipped" };
            let stderr = format!("verify 命令被命令安全网关拒绝：{reason}");
            let _ = db.record_task_verification(
                &task.id,
                task.run_id.as_deref(),
                &entry.kind,
                &entry.command,
                None,
                status_label,
                "",
                &stderr,
                now,
                Some(now),
            );
            reports.push(AutoVerifyReport {
                command: entry.command.clone(),
                passed: false,
                exit_code: None,
                stderr_tail: tail(&stderr),
                required: entry.required,
            });
            continue;
        }
        let started_at = now_ms();
        let (stdout, stderr, exit_code_raw, timed_out) =
            execute_command(&entry.command, cwd.as_deref()).await;
        let finished_at = now_ms();
        let exit_code: Option<i64> = exit_code_raw.map(|c| c as i64);
        let status_label = if timed_out {
            "timeout"
        } else if exit_code_raw == Some(0) {
            "passed"
        } else {
            "failed"
        };
        let _ = db.record_task_verification(
            &task.id,
            task.run_id.as_deref(),
            "auto_command",
            &entry.command,
            exit_code,
            status_label,
            &tail(&stdout),
            &tail(&stderr),
            started_at,
            Some(finished_at),
        );
        reports.push(AutoVerifyReport {
            command: entry.command.clone(),
            passed: status_label == "passed",
            exit_code,
            stderr_tail: tail(&stderr),
            required: entry.required,
        });
    }
    if reports.is_empty() {
        None
    } else {
        Some(reports)
    }
}

async fn execute_command(
    command: &str,
    cwd: Option<&std::path::Path>,
) -> (String, String, Option<i32>, bool) {
    let (program, args) = if cfg!(windows) {
        ("cmd", vec!["/C", command])
    } else {
        ("sh", vec!["-c", command])
    };
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return (String::new(), format!("spawn failed: {e}"), None, false);
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_buf = drain(stdout);
    let stderr_buf = drain(stderr);

    let wait = async {
        let (status, out, err) = tokio::join!(child.wait(), stdout_buf, stderr_buf);
        (
            status.ok().and_then(|s| s.code()),
            out.unwrap_or_default(),
            err.unwrap_or_default(),
        )
    };

    match timeout(Duration::from_secs(VERIFY_TIMEOUT_SECS), wait).await {
        Ok((code, out, err)) => (out, err, code, false),
        Err(_) => (String::new(), "verify timed out".to_string(), None, true),
    }
}

async fn drain<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    stream: Option<R>,
) -> std::io::Result<String> {
    if let Some(mut s) = stream {
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await?;
        Ok(String::from_utf8_lossy(&buf).to_string())
    } else {
        Ok(String::new())
    }
}

fn tail(s: &str) -> String {
    if s.chars().count() <= TAIL_CHARS {
        s.to_string()
    } else {
        let start = s.chars().count() - TAIL_CHARS;
        s.chars().skip(start).collect()
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db() -> LocalDb {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        LocalDb::open(std::env::temp_dir().join(format!("aura_runverify_{unique}.db"))).unwrap()
    }

    #[tokio::test]
    async fn verify_after_command_runs_and_records_auto_command_evidence() {
        // P2-1: a matcher-gated post-command verify runs the active task's verify
        // and lands an evidence row (source "auto_command"), without flipping the
        // task status.
        let db = temp_db();
        let session = db.create_session("s-verify").unwrap();
        let task = db
            .create_plan_task_full(
                &session.id,
                "P2-1 probe",
                None,
                None,
                "test",
                None,
                Some(&json!(["echo verify-ok"])),
            )
            .unwrap();

        let reports = verify_after_command(&db, &task, None)
            .await
            .expect("verify ran");
        assert_eq!(reports.len(), 1);
        assert!(reports[0].passed);
        assert_eq!(reports[0].command, "echo verify-ok");
        assert!(reports[0].required);

        // Evidence must land in run_task_verifications (假完成红线: 不落 evidence).
        let rows = db.list_task_verifications(&task.id).unwrap();
        assert!(rows
            .iter()
            .any(|row| row.status == "passed" && row.command == "echo verify-ok"));

        // Status must NOT be flipped by a mid-run command verify (that's done-time's job).
        let after = db.get_plan_task(&task.id).unwrap();
        assert_ne!(after.status, "blocked");
    }

    #[tokio::test]
    async fn verify_after_command_runs_all_entries_and_flags_optional() {
        // P2-2: mid-run safety net runs EVERY verify entry (not just the first)
        // and feeds back a per-entry verdict, flagging non-required ones.
        let db = temp_db();
        let session = db.create_session("s-verify-all").unwrap();
        let task = db
            .create_plan_task_full(
                &session.id,
                "P2-2 mid-run",
                None,
                None,
                "test",
                None,
                Some(&json!(["echo a", { "command": "exit 1", "required": false }])),
            )
            .unwrap();

        let reports = verify_after_command(&db, &task, None)
            .await
            .expect("verify ran");
        assert_eq!(reports.len(), 2);
        assert!(reports[0].passed && reports[0].required);
        assert!(!reports[1].passed && !reports[1].required);

        // Both entries landed evidence.
        let rows = db.list_task_verifications(&task.id).unwrap();
        assert!(rows
            .iter()
            .any(|r| r.command == "echo a" && r.status == "passed"));
        assert!(rows
            .iter()
            .any(|r| r.command == "exit 1" && r.status != "passed"));
    }

    #[tokio::test]
    async fn verify_after_command_returns_none_without_verify_entry() {
        let db = temp_db();
        let session = db.create_session("s-empty").unwrap();
        let task = db
            .create_plan_task_full(&session.id, "no verify", None, None, "test", None, None)
            .unwrap();
        assert!(verify_after_command(&db, &task, None).await.is_none());
    }

    #[test]
    fn all_verify_entries_parses_strings_and_objects() {
        let entries = all_verify_entries(&json!([
            "echo a",
            "  ",
            { "command": "echo b", "required": false, "kind": "frontend_build" },
            { "command": "echo c" },
            { "command": "  " },
            42
        ]));
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].command, "echo a");
        assert!(entries[0].required); // string defaults to required
        assert_eq!(entries[1].command, "echo b");
        assert!(!entries[1].required);
        assert_eq!(entries[1].kind, "frontend_build");
        assert_eq!(entries[2].command, "echo c");
        assert!(entries[2].required); // object without `required` defaults true
    }

    #[tokio::test]
    async fn auto_verify_done_runs_all_entries_and_records_each() {
        // P2-2 假完成红线: 只跑第一条仍标 done。这里两条都必须执行并各自落 evidence。
        let db = temp_db();
        let session = db.create_session("s-all").unwrap();
        let task = db
            .create_plan_task_full(
                &session.id,
                "multi",
                None,
                None,
                "test",
                None,
                Some(&json!([
                    "echo one",
                    { "command": "echo two", "kind": "frontend_build" }
                ])),
            )
            .unwrap();

        let outcome = auto_verify_done_task(&db, &task, None).await.expect("ran");
        assert!(outcome.passed);
        assert!(!outcome.blocked);

        let rows = db.list_task_verifications(&task.id).unwrap();
        assert!(rows
            .iter()
            .any(|r| r.command == "echo one" && r.status == "passed"));
        assert!(rows
            .iter()
            .any(|r| r.command == "echo two" && r.status == "passed"));
        assert!(rows
            .iter()
            .any(|r| r.command == "echo two" && r.kind == "frontend_build"));
    }

    #[tokio::test]
    async fn auto_verify_done_fails_when_required_entry_fails() {
        // 第二条必需项失败 → 任务整体不算通过（passed=false），但两条都已执行落 evidence。
        let db = temp_db();
        let session = db.create_session("s-req").unwrap();
        let task = db
            .create_plan_task_full(
                &session.id,
                "req-fail",
                None,
                None,
                "test",
                None,
                Some(&json!(["echo ok", "exit 1"])),
            )
            .unwrap();

        let outcome = auto_verify_done_task(&db, &task, None).await.expect("ran");
        assert!(!outcome.passed);

        let rows = db.list_task_verifications(&task.id).unwrap();
        assert!(rows
            .iter()
            .any(|r| r.command == "echo ok" && r.status == "passed"));
        assert!(rows
            .iter()
            .any(|r| r.command == "exit 1" && r.status != "passed"));
    }

    #[tokio::test]
    async fn auto_verify_done_denied_required_entry_fails_and_records_evidence() {
        // A required verify blocked by the command safety gateway is not a no-op:
        // it must fail the aggregate and leave an audit row.
        let db = temp_db();
        let session = db.create_session("s-denied").unwrap();
        let task = db
            .create_plan_task_full(
                &session.id,
                "denied-required",
                None,
                None,
                "test",
                None,
                Some(&json!([{ "command": "rm -rf /", "kind": "security_gate" }])),
            )
            .unwrap();

        let outcome = auto_verify_done_task(&db, &task, None).await.expect("ran");
        assert!(!outcome.passed);

        let rows = db.list_task_verifications(&task.id).unwrap();
        assert!(rows.iter().any(|r| {
            r.command == "rm -rf /"
                && r.kind == "security_gate"
                && r.status == "failed"
                && r.stderr_tail.contains("安全网关拒绝")
        }));
    }

    #[tokio::test]
    async fn auto_verify_done_passes_when_only_optional_fails() {
        // 标记为非必需的条目失败不应阻断 done（卡: 支持标记非必需）。
        let db = temp_db();
        let session = db.create_session("s-opt").unwrap();
        let task = db
            .create_plan_task_full(
                &session.id,
                "opt-fail",
                None,
                None,
                "test",
                None,
                Some(&json!(["echo ok", { "command": "exit 1", "required": false }])),
            )
            .unwrap();

        let outcome = auto_verify_done_task(&db, &task, None).await.expect("ran");
        assert!(outcome.passed);

        let rows = db.list_task_verifications(&task.id).unwrap();
        assert!(rows
            .iter()
            .any(|r| r.command == "exit 1" && r.status != "passed"));
    }
}
