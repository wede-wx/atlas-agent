use crate::storage::{RunTimeline, RunTimelineEntry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatusSemantic {
    pub domain: String,
    pub status: String,
    pub tone: String,
    pub label: String,
    pub is_terminal: bool,
    pub blocks_completion: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunTerminalFeed {
    pub run_id: String,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
    pub entries: Vec<RunTerminalEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunTerminalEntry {
    pub id: String,
    pub source_id: String,
    pub source_kind: String,
    pub at: i64,
    pub finished_at: Option<i64>,
    pub seq: i64,
    pub command: String,
    pub cwd: Option<String>,
    pub shell: Option<String>,
    pub status: String,
    pub exit_code: Option<i64>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub summary: String,
    pub tool_call_id: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunAuditFeed {
    pub run_id: String,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
    pub entries: Vec<RunAuditEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunAuditEntry {
    pub id: String,
    pub source_id: String,
    pub source_kind: String,
    pub at: i64,
    pub finished_at: Option<i64>,
    pub seq: i64,
    pub category: String,
    pub label: String,
    pub status: Option<String>,
    pub semantic: StatusSemantic,
    pub risk: Option<String>,
    pub actor: Option<String>,
    pub reason: Option<String>,
    pub detail: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunProgressSummary {
    pub run_id: String,
    pub run_status: Option<String>,
    pub stage: String,
    pub semantic: StatusSemantic,
    pub latest_message: Option<String>,
    pub event_counts: BTreeMap<String, i64>,
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub last_event_at: Option<i64>,
    pub terminal: bool,
    pub has_verification: bool,
    pub failed_verification_count: i64,
    pub pending_permission_count: i64,
}

pub fn status_semantic(domain: &str, status: &str) -> StatusSemantic {
    let domain = normalize_token(domain);
    let status = normalize_token(status);
    let (tone, label, is_terminal, blocks_completion) = match domain.as_str() {
        "run" => match status.as_str() {
            "finished" | "complete" | "completed" => ("success", "finished", true, false),
            "failed" | "cancelled" | "canceled" => ("danger", status.as_str(), true, true),
            "blocked" => ("danger", "blocked", true, true),
            "paused" => ("warning", "paused", false, false),
            "running" => ("info", "running", false, false),
            _ => ("neutral", status.as_str(), false, false),
        },
        "task" => match status.as_str() {
            "verified" | "passed" => ("success", status.as_str(), true, false),
            "done" | "completed" | "finished" => ("info", status.as_str(), true, false),
            "verifying" | "running" | "doing" | "waiting" | "pending" => {
                ("info", status.as_str(), false, false)
            }
            "waived" | "skipped" => ("muted", status.as_str(), true, false),
            "failed" | "blocked" => ("danger", status.as_str(), true, true),
            _ => ("neutral", status.as_str(), false, false),
        },
        "verify" | "verification" => match status.as_str() {
            "passed" | "verified" => ("success", status.as_str(), true, false),
            "failed" | "error" => ("danger", status.as_str(), true, true),
            "skipped" | "waived" => ("warning", status.as_str(), true, false),
            "running" | "pending" => ("info", status.as_str(), false, false),
            _ => ("neutral", status.as_str(), false, false),
        },
        "permission" => match status.as_str() {
            "allowed" | "approved" => ("success", status.as_str(), true, false),
            "needs_confirm" | "needs-confirm" | "pending" => {
                ("warning", "needs_confirm", false, true)
            }
            "denied" | "rejected" => ("danger", status.as_str(), true, true),
            _ => ("neutral", status.as_str(), false, false),
        },
        "delivery" | "final_audit" => match status.as_str() {
            "complete" | "completed" | "verified" => ("success", status.as_str(), true, false),
            "partial" | "unverified" | "unknown" => ("warning", status.as_str(), true, true),
            "blocked" | "failed" => ("danger", status.as_str(), true, true),
            _ => ("neutral", status.as_str(), false, false),
        },
        "audit" | "tool" => match status.as_str() {
            "allowed" | "completed" | "finished" | "success" | "ok" => {
                ("success", status.as_str(), true, false)
            }
            "pending" | "running" | "needs_confirm" => ("warning", status.as_str(), false, false),
            "denied" | "failed" | "error" | "blocked" => ("danger", status.as_str(), true, true),
            _ => ("neutral", status.as_str(), false, false),
        },
        _ => match status.as_str() {
            "failed" | "error" | "denied" | "blocked" => ("danger", status.as_str(), true, true),
            "passed" | "verified" | "allowed" => ("success", status.as_str(), true, false),
            "running" | "pending" => ("info", status.as_str(), false, false),
            _ => ("neutral", status.as_str(), false, false),
        },
    };

    let label = label.to_string();
    StatusSemantic {
        domain,
        status,
        tone: tone.to_string(),
        label,
        is_terminal,
        blocks_completion,
    }
}

pub fn run_terminal_feed(timeline: &RunTimeline, limit: i64, offset: i64) -> RunTerminalFeed {
    let entries: Vec<RunTerminalEntry> = timeline
        .entries
        .iter()
        .filter_map(terminal_entry_from_timeline)
        .collect();
    let (total, offset, limit, page) = paginate(entries, limit, offset);
    RunTerminalFeed {
        run_id: timeline.run_id.clone(),
        total,
        offset,
        limit,
        entries: page,
    }
}

pub fn run_audit_feed(timeline: &RunTimeline, limit: i64, offset: i64) -> RunAuditFeed {
    let entries: Vec<RunAuditEntry> = timeline
        .entries
        .iter()
        .filter_map(audit_entry_from_timeline)
        .collect();
    let (total, offset, limit, page) = paginate(entries, limit, offset);
    RunAuditFeed {
        run_id: timeline.run_id.clone(),
        total,
        offset,
        limit,
        entries: page,
    }
}

pub fn run_progress_summary(timeline: &RunTimeline) -> RunProgressSummary {
    let mut event_counts = BTreeMap::new();
    let mut latest_message = None;
    let mut last_event_at = None;
    let mut has_verification = false;
    let mut failed_verification_count = 0;
    let mut pending_permission_count = 0;

    for entry in &timeline.entries {
        *event_counts.entry(entry.kind.clone()).or_insert(0) += 1;
        if last_event_at.map(|at| entry.at >= at).unwrap_or(true) {
            last_event_at = Some(entry.at);
            latest_message = latest_entry_message(entry);
        }
        if entry.kind == "verify" {
            has_verification = true;
            let status = normalize_token(entry.status.as_deref().unwrap_or_default());
            if matches!(status.as_str(), "failed" | "error") {
                failed_verification_count += 1;
            }
        }
        if entry.kind == "permission" {
            let status = normalize_token(entry.status.as_deref().unwrap_or_default());
            if matches!(
                status.as_str(),
                "needs_confirm" | "needs-confirm" | "pending"
            ) {
                pending_permission_count += 1;
            }
        }
    }

    let run_status = timeline.run.as_ref().map(|run| run.status.clone());
    let stage = derive_stage(
        timeline,
        pending_permission_count,
        failed_verification_count,
    );
    let semantic = status_semantic("run", run_status.as_deref().unwrap_or(&stage));
    let terminal = semantic.is_terminal
        || timeline
            .run
            .as_ref()
            .and_then(|run| run.finished_at)
            .is_some();

    RunProgressSummary {
        run_id: timeline.run_id.clone(),
        run_status,
        stage,
        semantic,
        latest_message,
        event_counts,
        started_at: timeline.run.as_ref().map(|run| run.created_at),
        updated_at: timeline.run.as_ref().map(|run| run.updated_at),
        finished_at: timeline.run.as_ref().and_then(|run| run.finished_at),
        last_event_at,
        terminal,
        has_verification,
        failed_verification_count,
        pending_permission_count,
    }
}

fn terminal_entry_from_timeline(entry: &RunTimelineEntry) -> Option<RunTerminalEntry> {
    if entry.kind != "step" && entry.kind != "verify" {
        return None;
    }

    let detail = &entry.detail;
    let input = detail.get("input").unwrap_or(&Value::Null);
    let output = detail.get("output").unwrap_or(&Value::Null);

    let command = first_string(&[
        get_string(detail, &["command"]),
        get_string(output, &["data", "pendingCommand", "command"]),
        get_string(output, &["pendingCommand", "command"]),
        get_string(input, &["command"]),
        get_string(input, &["cmd"]),
        get_string(input, &["args", "command"]),
        get_string(input, &["arguments", "command"]),
        get_string(output, &["command"]),
        get_string(output, &["cmd"]),
        get_string(output, &["data", "command"]),
        get_string(output, &["result", "command"]),
        get_string(output, &["data", "result", "command"]),
    ])?;

    Some(RunTerminalEntry {
        id: format!("terminal:{}", entry.id),
        source_id: entry.id.clone(),
        source_kind: entry.kind.clone(),
        at: entry.at,
        finished_at: entry.finished_at,
        seq: entry.seq,
        command,
        cwd: first_string(&[
            get_string(output, &["data", "pendingCommand", "cwd"]),
            get_string(output, &["pendingCommand", "cwd"]),
            get_string(input, &["cwd"]),
            get_string(input, &["workdir"]),
            get_string(output, &["cwd"]),
            get_string(output, &["workdir"]),
        ]),
        shell: first_string(&[
            get_string(output, &["data", "pendingCommand", "shell"]),
            get_string(output, &["pendingCommand", "shell"]),
            get_string(input, &["shell"]),
            get_string(output, &["shell"]),
        ]),
        status: entry
            .status
            .clone()
            .or_else(|| get_string(detail, &["status"]))
            .unwrap_or_else(|| "finished".to_string()),
        exit_code: first_i64(&[
            get_i64(detail, &["exitCode"]),
            get_i64(output, &["exitCode"]),
            get_i64(output, &["exit_code"]),
            get_i64(output, &["data", "exitCode"]),
            get_i64(output, &["result", "exitCode"]),
            get_i64(output, &["data", "result", "exitCode"]),
        ]),
        stdout_tail: first_string(&[
            get_string(detail, &["stdoutTail"]),
            get_string(output, &["stdoutTail"]),
            get_string(output, &["stdout"]),
            get_string(output, &["data", "stdoutTail"]),
            get_string(output, &["data", "stdout"]),
            get_string(output, &["result", "stdoutTail"]),
            get_string(output, &["result", "stdout"]),
        ]),
        stderr_tail: first_string(&[
            get_string(detail, &["stderrTail"]),
            get_string(output, &["stderrTail"]),
            get_string(output, &["stderr"]),
            get_string(output, &["data", "stderrTail"]),
            get_string(output, &["data", "stderr"]),
            get_string(output, &["result", "stderrTail"]),
            get_string(output, &["result", "stderr"]),
        ]),
        summary: get_string(detail, &["summary"]).unwrap_or_else(|| entry.label.clone()),
        tool_call_id: first_string(&[
            get_string(detail, &["toolCallId"]),
            get_string(input, &["toolCallId"]),
            get_string(input, &["id"]),
        ]),
        truncated: first_bool(&[
            get_bool(output, &["truncated"]),
            get_bool(output, &["stdoutTruncated"]),
            get_bool(output, &["stderrTruncated"]),
            get_bool(output, &["data", "truncated"]),
        ])
        .unwrap_or(false),
    })
}

fn audit_entry_from_timeline(entry: &RunTimelineEntry) -> Option<RunAuditEntry> {
    let category = match entry.kind.as_str() {
        "tool" => "tool_audit",
        "permission" => "permission",
        "verify" => "verification",
        "plan_change" => "plan_change",
        "usage" => "usage",
        "step" if has_final_audit_payload(&entry.detail) => "final_audit",
        _ => return None,
    };

    let status = entry
        .status
        .clone()
        .or_else(|| get_string(&entry.detail, &["status"]))
        .or_else(|| get_string(&entry.detail, &["finalAuditStatus"]));
    let semantic_domain = match category {
        "permission" => "permission",
        "verification" => "verify",
        "final_audit" => "final_audit",
        "tool_audit" => "tool",
        _ => "audit",
    };
    let semantic = status_semantic(semantic_domain, status.as_deref().unwrap_or("recorded"));

    Some(RunAuditEntry {
        id: format!("audit:{}", entry.id),
        source_id: entry.id.clone(),
        source_kind: entry.kind.clone(),
        at: entry.at,
        finished_at: entry.finished_at,
        seq: entry.seq,
        category: category.to_string(),
        label: entry.label.clone(),
        status,
        semantic,
        risk: get_string(&entry.detail, &["risk"]),
        actor: first_string(&[
            get_string(&entry.detail, &["actor"]),
            get_string(&entry.detail, &["decidedBy"]),
            get_string(&entry.detail, &["policy"]),
        ]),
        reason: first_string(&[
            get_string(&entry.detail, &["reason"]),
            get_string(&entry.detail, &["summary"]),
            get_string(&entry.detail, &["command"]),
        ]),
        detail: entry.detail.clone(),
    })
}

fn derive_stage(
    timeline: &RunTimeline,
    pending_permission_count: i64,
    failed_verification_count: i64,
) -> String {
    if let Some(run) = &timeline.run {
        let status = normalize_token(&run.status);
        if matches!(
            status.as_str(),
            "finished" | "failed" | "blocked" | "cancelled" | "canceled"
        ) {
            return status;
        }
    }
    if pending_permission_count > 0 {
        return "awaiting_permission".to_string();
    }
    if failed_verification_count > 0 {
        return "verification_failed".to_string();
    }
    if let Some(last) = timeline.entries.iter().max_by_key(|entry| entry.at) {
        if last.kind == "verify" {
            return "verifying".to_string();
        }
        if last.kind == "tool" || last.label == "tool_call" {
            return "tool_running".to_string();
        }
        if last.kind == "usage" {
            return "model_call".to_string();
        }
    }
    "running".to_string()
}

fn latest_entry_message(entry: &RunTimelineEntry) -> Option<String> {
    first_string(&[
        get_string(&entry.detail, &["summary"]),
        get_string(&entry.detail, &["reason"]),
        get_string(&entry.detail, &["command"]),
        Some(entry.label.clone()),
    ])
}

fn has_final_audit_payload(detail: &Value) -> bool {
    get_path(detail, &["output", "deliveryReport"]).is_some()
        || get_path(detail, &["output", "finalAudit"]).is_some()
        || get_path(detail, &["finalAudit"]).is_some()
}

fn normalize_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(' ', "_")
}

fn get_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn get_string(value: &Value, path: &[&str]) -> Option<String> {
    let raw = get_path(value, path)?;
    match raw {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn get_i64(value: &Value, path: &[&str]) -> Option<i64> {
    let raw = get_path(value, path)?;
    raw.as_i64()
        .or_else(|| raw.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
}

fn get_bool(value: &Value, path: &[&str]) -> Option<bool> {
    let raw = get_path(value, path)?;
    raw.as_bool().or_else(|| {
        raw.as_str()
            .map(|s| matches!(normalize_token(s).as_str(), "true" | "yes"))
    })
}

fn first_string(values: &[Option<String>]) -> Option<String> {
    values
        .iter()
        .filter_map(|value| value.as_ref())
        .find(|value| !value.trim().is_empty())
        .cloned()
}

fn first_i64(values: &[Option<i64>]) -> Option<i64> {
    values.iter().copied().flatten().next()
}

fn first_bool(values: &[Option<bool>]) -> Option<bool> {
    values.iter().copied().flatten().next()
}

fn paginate<T: Clone>(entries: Vec<T>, limit: i64, offset: i64) -> (i64, i64, i64, Vec<T>) {
    let total = entries.len() as i64;
    let limit = limit.clamp(1, 1000);
    let offset = offset.max(0).min(total);
    let start = offset as usize;
    let end = (start + limit as usize).min(entries.len());
    (total, offset, limit, entries[start..end].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(
        kind: &str,
        id: &str,
        at: i64,
        label: &str,
        status: Option<&str>,
        detail: Value,
    ) -> RunTimelineEntry {
        RunTimelineEntry {
            kind: kind.to_string(),
            id: id.to_string(),
            at,
            finished_at: None,
            seq: 1,
            label: label.to_string(),
            status: status.map(str::to_string),
            detail,
        }
    }

    fn timeline(entries: Vec<RunTimelineEntry>) -> RunTimeline {
        RunTimeline {
            run_id: "run-1".to_string(),
            run: None,
            total: entries.len() as i64,
            offset: 0,
            limit: entries.len() as i64,
            entries,
        }
    }

    #[test]
    fn terminal_feed_uses_real_command_payloads_only() {
        let timeline = timeline(vec![
            entry(
                "step",
                "s1",
                1,
                "approval",
                Some("finished"),
                json!({
                    "summary": "pending command",
                    "input": {},
                    "output": { "data": { "pendingCommand": {
                        "command": "cargo test --lib",
                        "cwd": "C:/repo",
                        "shell": "powershell"
                    }}}
                }),
            ),
            entry(
                "tool",
                "t1",
                2,
                "run_command",
                Some("finished"),
                json!({ "toolName": "run_command", "reason": "no command body" }),
            ),
        ]);

        let feed = run_terminal_feed(&timeline, 50, 0);
        assert_eq!(feed.total, 1);
        assert_eq!(feed.entries[0].command, "cargo test --lib");
        assert_eq!(feed.entries[0].cwd.as_deref(), Some("C:/repo"));
    }

    #[test]
    fn audit_feed_keeps_permission_risk_and_semantics() {
        let timeline = timeline(vec![entry(
            "permission",
            "p1",
            1,
            "rm -rf",
            Some("denied"),
            json!({
                "risk": "destructive",
                "decision": "denied",
                "reason": "unsafe command",
                "decidedBy": "policy"
            }),
        )]);

        let feed = run_audit_feed(&timeline, 50, 0);
        assert_eq!(feed.total, 1);
        assert_eq!(feed.entries[0].risk.as_deref(), Some("destructive"));
        assert_eq!(feed.entries[0].semantic.tone, "danger");
        assert!(feed.entries[0].semantic.blocks_completion);
    }

    #[test]
    fn task_done_is_not_colored_as_verified_success() {
        let done = status_semantic("task", "done");
        let verified = status_semantic("task", "verified");
        assert_eq!(done.tone, "info");
        assert_eq!(verified.tone, "success");
    }

    #[test]
    fn progress_summary_surfaces_permission_waiting() {
        let timeline = timeline(vec![entry(
            "permission",
            "p1",
            1,
            "write file",
            Some("needs_confirm"),
            json!({ "reason": "needs user confirmation" }),
        )]);

        let summary = run_progress_summary(&timeline);
        assert_eq!(summary.stage, "awaiting_permission");
        assert_eq!(summary.pending_permission_count, 1);
        assert_eq!(
            summary.latest_message.as_deref(),
            Some("needs user confirmation")
        );
    }
}
