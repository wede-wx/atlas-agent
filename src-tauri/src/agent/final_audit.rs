use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::storage::{
    FileCheckpointRecord, LocalDb, PlanTaskRecord, RunPlanRecord, StorageError,
    TaskVerificationRecord,
};

/// P3-4: a formalized goal — the user-facing **result** (observableOutcome) plus
/// explicit nonGoals, not just an action sentence. Built from a stored run plan.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub text: String,
    pub observable_outcome: Option<String>,
    pub non_goals: Vec<String>,
}

impl Goal {
    /// Build a Goal from a stored run plan, normalizing nonGoals to a string list.
    pub fn from_plan(plan: &RunPlanRecord) -> Self {
        Self {
            text: plan.goal.clone(),
            observable_outcome: plan.observable_outcome.clone(),
            non_goals: plan
                .non_goals
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// 一条 acceptance criterion 的审计状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionStatus {
    Passed,
    Failed,
    Waived,
    Pending,
}

impl CriterionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Waived => "waived",
            Self::Pending => "pending",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalAuditCriterion {
    pub text: String,
    pub status: CriterionStatus,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalAuditTask {
    pub id: String,
    pub title: String,
    pub status: String,
    pub evidence_status: String,
    pub active: bool,
    pub blocked_reason: Option<String>,
    /// 改动文件路径（M4 checkpoint 接入后填充，目前为空）。
    pub changed_files: Vec<String>,
    /// 关联的 verification id（目前为空，由 M3.4 repair loop 填充）。
    pub verification_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinalAuditStatus {
    /// 全部 task evidence_status=verified 且无 blocked。
    Completed,
    /// 至少一条 acceptance 被显式 waived，但其余 verified。
    CompletedWithWaiver,
    /// 至少一条 task 处于 blocked / failed。
    Blocked,
    /// 至少一条 task evidence_status=pending，但既未 failed 也未 blocked。
    Unverified,
}

impl FinalAuditStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::CompletedWithWaiver => "completed_with_waiver",
            Self::Blocked => "blocked",
            Self::Unverified => "unverified",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalAudit {
    pub goal: String,
    /// P3-4: observable result the goal must produce (None for legacy plans).
    #[serde(default)]
    pub observable_outcome: Option<String>,
    /// P3-4: explicit non-goals carried from the run plan.
    #[serde(default)]
    pub non_goals: Vec<String>,
    pub status: FinalAuditStatus,
    pub criteria: Vec<FinalAuditCriterion>,
    pub tasks: Vec<FinalAuditTask>,
    pub unverified: Vec<String>,
    pub risks: Vec<String>,
    pub mock_or_placeholder: Vec<String>,
}

impl FinalAudit {
    pub fn empty(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            observable_outcome: None,
            non_goals: Vec::new(),
            status: FinalAuditStatus::Completed,
            criteria: Vec::new(),
            tasks: Vec::new(),
            unverified: Vec::new(),
            risks: Vec::new(),
            mock_or_placeholder: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryPlan {
    pub id: Option<String>,
    pub status: Option<String>,
    pub acceptance_criteria: Vec<FinalAuditCriterion>,
    pub tasks: Vec<FinalAuditTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryChangedFile {
    pub path: String,
    pub task_id: Option<String>,
    pub checkpoint_id: String,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub before_size: i64,
    pub created_at: i64,
    pub restored_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryVerification {
    pub id: String,
    pub run_id: Option<String>,
    pub task_id: String,
    pub kind: String,
    pub command: String,
    pub exit_code: Option<i64>,
    pub status: String,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryReport {
    /// Backward-compatible alias used by the existing final-audit hard block.
    pub status: FinalAuditStatus,
    pub final_audit_status: FinalAuditStatus,
    pub goal: String,
    #[serde(default)]
    pub observable_outcome: Option<String>,
    #[serde(default)]
    pub non_goals: Vec<String>,
    pub plan: DeliveryPlan,
    pub completed: Vec<String>,
    pub changed_files: Vec<DeliveryChangedFile>,
    pub verification: Vec<DeliveryVerification>,
    pub unverified: Vec<String>,
    pub remaining_risks: Vec<String>,
    pub mock_or_placeholder: Vec<String>,
    pub final_audit: FinalAudit,
}

/// 根据当前 session 的 plan_tasks 生成 final audit。
pub fn compute_final_audit(
    db: &LocalDb,
    session_id: &str,
    goal: impl Into<String>,
) -> Result<FinalAudit, StorageError> {
    let tasks = db.list_plan_tasks(session_id)?;
    let mut audit = FinalAudit::empty(goal);
    audit.tasks = tasks.iter().map(task_to_audit).collect();

    // T7.4 + P3-4 — populate goal object and criteria from the session's latest
    // run_plan. The stored plan goal (a formalized Goal with observableOutcome +
    // nonGoals) overrides the raw user-input fallback so final audit judges
    // against the recorded result, not just the request sentence.
    if let Ok(plans) = db.list_run_plans(session_id) {
        if let Some(plan) = plans.into_iter().next() {
            let goal = Goal::from_plan(&plan);
            if !goal.text.is_empty() {
                audit.goal = goal.text;
            }
            audit.observable_outcome = goal.observable_outcome;
            audit.non_goals = goal.non_goals;
            audit.criteria = criteria_from_plan(&plan.acceptance_criteria, &tasks);
        }
    }

    let mut has_blocked = false;
    let mut has_failed_evidence = false;
    let mut has_pending = false;
    let mut has_waiver = false;
    let mut all_verified_or_waived = !tasks.is_empty();

    for task in &tasks {
        let status_lc = task.status.to_ascii_lowercase();
        let evidence_lc = task.evidence_status.to_ascii_lowercase();
        if status_lc == "blocked" {
            has_blocked = true;
            audit.unverified.push(format!("{} - blocked", task.title));
        }
        match evidence_lc.as_str() {
            "verified" => {}
            "waived" => has_waiver = true,
            "failed" => {
                has_failed_evidence = true;
                audit
                    .unverified
                    .push(format!("{} - evidence_status=failed", task.title));
                all_verified_or_waived = false;
            }
            "pending" => {
                has_pending = true;
                audit
                    .unverified
                    .push(format!("{} - evidence_status=pending", task.title));
                all_verified_or_waived = false;
            }
            _ => {
                if status_lc == "done" {
                    // done 但没有 evidence_status 记录的兜底
                    has_pending = true;
                    audit
                        .unverified
                        .push(format!("{} - evidence_status=none", task.title));
                    all_verified_or_waived = false;
                }
            }
        }
    }

    audit.status = if has_blocked || has_failed_evidence {
        FinalAuditStatus::Blocked
    } else if has_pending {
        FinalAuditStatus::Unverified
    } else if tasks.is_empty() {
        // Empty session has nothing to verify — we cannot claim Completed
        // honestly, so surface it as Unverified per plan §M3.5.
        FinalAuditStatus::Unverified
    } else if all_verified_or_waived && has_waiver {
        FinalAuditStatus::CompletedWithWaiver
    } else {
        FinalAuditStatus::Completed
    };

    if tasks.is_empty() {
        audit
            .unverified
            .push("没有任何已记录的计划任务。".to_string());
    }

    Ok(audit)
}

/// P2-13: build the user-facing delivery report from auditable storage rows.
///
/// FinalAudit decides whether the plan is honestly complete. DeliveryReport then
/// attaches the accountable evidence surface: changed files from checkpoints and
/// verification rows from `run_task_verifications`. A completed audit without
/// any real verification row is downgraded to `unverified`; otherwise the report
/// would be indistinguishable from a hand-filled status flag.
pub fn compute_delivery_report(
    db: &LocalDb,
    session_id: &str,
    run_id: Option<&str>,
    goal: impl Into<String>,
) -> Result<DeliveryReport, StorageError> {
    let mut audit = compute_final_audit(db, session_id, goal)?;
    let latest_plan = db.list_run_plans(session_id)?.into_iter().next();
    let checkpoints = match run_id {
        Some(run_id) => db.list_file_checkpoints_by_run(run_id)?,
        None => Vec::new(),
    };
    let verifications = match run_id {
        Some(run_id) => db.list_task_verifications_by_run(run_id)?,
        None => verification_rows_for_tasks(db, &audit.tasks)?,
    };

    attach_task_delivery_refs(&mut audit.tasks, &checkpoints, &verifications);
    harden_delivery_status(&mut audit, &checkpoints, &verifications);

    let completed = audit
        .tasks
        .iter()
        .filter(|task| {
            task.status.eq_ignore_ascii_case("done")
                && matches!(task.evidence_status.as_str(), "verified" | "waived")
        })
        .map(|task| task.title.clone())
        .collect::<Vec<_>>();

    let plan = DeliveryPlan {
        id: latest_plan.as_ref().map(|plan| plan.id.clone()),
        status: latest_plan.as_ref().map(|plan| plan.status.clone()),
        acceptance_criteria: audit.criteria.clone(),
        tasks: audit.tasks.clone(),
    };

    let status = audit.status;
    Ok(DeliveryReport {
        status,
        final_audit_status: status,
        goal: audit.goal.clone(),
        observable_outcome: audit.observable_outcome.clone(),
        non_goals: audit.non_goals.clone(),
        plan,
        completed,
        changed_files: delivery_changed_files(&checkpoints),
        verification: verifications
            .into_iter()
            .map(delivery_verification)
            .collect(),
        unverified: audit.unverified.clone(),
        remaining_risks: audit.risks.clone(),
        mock_or_placeholder: audit.mock_or_placeholder.clone(),
        final_audit: audit,
    })
}

pub fn delivery_report_text(report: &serde_json::Value) -> Option<String> {
    let status = report_status(report).unwrap_or("unknown");
    let completed = string_array(report, "completed")
        .or_else(|| completed_from_legacy_tasks(report))
        .unwrap_or_default();
    let changed_files = changed_file_lines(report);
    let verification = verification_lines(report);
    let unverified = string_array(report, "unverified").unwrap_or_default();
    let risks = string_array(report, "remainingRisks")
        .or_else(|| string_array(report, "risks"))
        .unwrap_or_default();
    let mocks = string_array(report, "mockOrPlaceholder")
        .or_else(|| string_array(report, "mock_or_placeholder"))
        .unwrap_or_default();

    let mut lines = vec![format!("[Atlas 交付报告] status={status}")];
    push_section(&mut lines, "已完成", completed, "无已完成任务记录。");
    push_section(
        &mut lines,
        "变更文件",
        changed_files,
        "无 checkpoint 变更文件记录。",
    );
    push_section(
        &mut lines,
        "已验证",
        verification,
        "无 run_task_verifications 记录。",
    );
    push_section(&mut lines, "未验证", unverified, "无。");
    push_section(&mut lines, "风险", risks, "无。");
    push_section(&mut lines, "Mock/占位", mocks, "无。");
    Some(lines.join("\n"))
}

pub fn report_status(report: &serde_json::Value) -> Option<&str> {
    report
        .get("finalAuditStatus")
        .or_else(|| report.get("status"))
        .and_then(|value| value.as_str())
}

/// P2-3: when final audit status is `blocked`/`unverified`, return a guard banner
/// that MUST be prepended to the model's prose so the user's takeaway can never be
/// 「已完成」. The banner explicitly negates any completion claim in the body.
/// Returns `None` for `completed`/`completed_with_waiver` (no interception). This
/// is the physical hard block (§7.8): a footer alone let the model still open with
/// "已完成"; a leading banner makes that impossible.
pub fn completion_guard_prefix(status: &str) -> Option<String> {
    match status {
        "blocked" => Some(
            "⚠️ 本次任务被阻断（blocked），尚未完成。以下内容仅为过程说明，不代表任务已完成或已通过验证。"
                .to_string(),
        ),
        "unverified" => Some(
            "⚠️ 本次任务未通过验证（unverified）。以下内容不代表任务已完成；在验证通过前，请勿当作「已完成」。"
                .to_string(),
        ),
        _ => None,
    }
}

/// Build a list of audit criteria from a plan's acceptance_criteria JSON value.
///
/// Accepts either `Array<String>` or `Array<{ text: String, ... }>`. Unknown
/// shapes degrade to a single placeholder criterion so the audit still surfaces
/// that criteria existed but couldn't be parsed.
fn criteria_from_plan(
    raw: &serde_json::Value,
    tasks: &[PlanTaskRecord],
) -> Vec<FinalAuditCriterion> {
    let mut out = Vec::new();
    let Some(items) = raw.as_array() else {
        return out;
    };
    let all_verified = !tasks.is_empty()
        && tasks
            .iter()
            .all(|t| matches!(t.evidence_status.as_str(), "verified" | "waived"));
    let has_blocked_or_failed = tasks.iter().any(|t| {
        t.status.eq_ignore_ascii_case("blocked") || matches!(t.evidence_status.as_str(), "failed")
    });

    for item in items {
        let text = match item {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(map) => map
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("(criterion has no `text` field)")
                .to_string(),
            other => other.to_string(),
        };
        let status = if has_blocked_or_failed {
            CriterionStatus::Failed
        } else if all_verified {
            CriterionStatus::Passed
        } else {
            CriterionStatus::Pending
        };
        out.push(FinalAuditCriterion {
            text,
            status,
            evidence: None,
        });
    }
    out
}

fn task_to_audit(task: &PlanTaskRecord) -> FinalAuditTask {
    FinalAuditTask {
        id: task.id.clone(),
        title: task.title.clone(),
        status: task.status.clone(),
        evidence_status: task.evidence_status.clone(),
        active: task.active,
        blocked_reason: task.blocked_reason.clone(),
        changed_files: Vec::new(),
        verification_ids: Vec::new(),
    }
}

fn verification_rows_for_tasks(
    db: &LocalDb,
    tasks: &[FinalAuditTask],
) -> Result<Vec<TaskVerificationRecord>, StorageError> {
    let mut rows = Vec::new();
    for task in tasks {
        rows.extend(db.list_task_verifications(&task.id)?);
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.started_at));
    Ok(rows)
}

fn attach_task_delivery_refs(
    tasks: &mut [FinalAuditTask],
    checkpoints: &[FileCheckpointRecord],
    verifications: &[TaskVerificationRecord],
) {
    let mut changed_by_task: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for checkpoint in checkpoints {
        let Some(task_id) = checkpoint.task_id.as_ref() else {
            continue;
        };
        changed_by_task
            .entry(task_id.clone())
            .or_default()
            .insert(checkpoint.path.clone());
    }

    let mut verification_by_task: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for verification in verifications {
        verification_by_task
            .entry(verification.task_id.clone())
            .or_default()
            .insert(verification.id.clone());
    }

    for task in tasks {
        task.changed_files = changed_by_task
            .remove(&task.id)
            .map(|paths| paths.into_iter().collect())
            .unwrap_or_default();
        task.verification_ids = verification_by_task
            .remove(&task.id)
            .map(|ids| ids.into_iter().collect())
            .unwrap_or_default();
    }
}

fn harden_delivery_status(
    audit: &mut FinalAudit,
    checkpoints: &[FileCheckpointRecord],
    verifications: &[TaskVerificationRecord],
) {
    if checkpoints.is_empty() {
        audit.risks.push(
            "本 run 没有 checkpoint 记录；若任务涉及文件修改，无法从 checkpoint 对账 changed files。"
                .to_string(),
        );
    }

    if verifications.is_empty()
        && matches!(
            audit.status,
            FinalAuditStatus::Completed | FinalAuditStatus::CompletedWithWaiver
        )
    {
        audit.status = FinalAuditStatus::Unverified;
        audit
            .unverified
            .push("没有 run_task_verifications 记录，无法证明验证命令真实执行。".to_string());
        audit.risks.push(
            "DeliveryReport 未找到真实 verification 记录；不能按 completed 交付。".to_string(),
        );
    }
}

fn delivery_changed_files(checkpoints: &[FileCheckpointRecord]) -> Vec<DeliveryChangedFile> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for checkpoint in checkpoints {
        if !seen.insert(checkpoint.path.clone()) {
            continue;
        }
        out.push(DeliveryChangedFile {
            path: checkpoint.path.clone(),
            task_id: checkpoint.task_id.clone(),
            checkpoint_id: checkpoint.id.clone(),
            before_hash: checkpoint.before_hash.clone(),
            after_hash: checkpoint.after_hash.clone(),
            before_size: checkpoint.before_size,
            created_at: checkpoint.created_at,
            restored_at: checkpoint.restored_at,
        });
    }
    out
}

fn delivery_verification(row: TaskVerificationRecord) -> DeliveryVerification {
    DeliveryVerification {
        id: row.id,
        run_id: row.run_id,
        task_id: row.task_id,
        kind: row.kind,
        command: row.command,
        exit_code: row.exit_code,
        status: row.status,
        stdout_tail: row.stdout_tail,
        stderr_tail: row.stderr_tail,
        started_at: row.started_at,
        finished_at: row.finished_at,
    }
}

fn string_array(report: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    report.get(key)?.as_array().map(|items| {
        items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect()
    })
}

fn completed_from_legacy_tasks(report: &serde_json::Value) -> Option<Vec<String>> {
    let tasks = report.get("tasks")?.as_array()?;
    Some(
        tasks
            .iter()
            .filter(|task| {
                task.get("status")
                    .and_then(|value| value.as_str())
                    .is_some_and(|status| status.eq_ignore_ascii_case("done"))
            })
            .filter_map(|task| task.get("title").and_then(|value| value.as_str()))
            .map(str::to_string)
            .collect(),
    )
}

fn changed_file_lines(report: &serde_json::Value) -> Vec<String> {
    report
        .get("changedFiles")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_str().map(str::to_string).or_else(|| {
                        item.get("path")
                            .and_then(|value| value.as_str())
                            .map(str::to_string)
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn verification_lines(report: &serde_json::Value) -> Vec<String> {
    report
        .get("verification")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let status = item.get("status").and_then(|value| value.as_str())?;
                    let command = item
                        .get("command")
                        .and_then(|value| value.as_str())
                        .unwrap_or("(no command)");
                    let kind = item
                        .get("kind")
                        .and_then(|value| value.as_str())
                        .unwrap_or("verify");
                    Some(format!("{kind}: `{command}` -> {status}"))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn push_section(lines: &mut Vec<String>, title: &str, items: Vec<String>, empty: &str) {
    lines.push(format!("{title}："));
    if items.is_empty() {
        lines.push(format!("- {empty}"));
        return;
    }
    for item in items.into_iter().take(12) {
        lines.push(format!("- {item}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn fresh_db() -> (std::path::PathBuf, LocalDb, String) {
        let path = std::env::temp_dir().join(format!("atlas_final_audit_{}.db", Uuid::new_v4()));
        let db = LocalDb::open(path.clone()).expect("open");
        let session = db.create_session("final-audit-test").expect("session");
        (path, db, session.id)
    }

    #[test]
    fn empty_session_marks_unverified() {
        let (_path, db, session_id) = fresh_db();
        let audit = compute_final_audit(&db, &session_id, "测试目标").expect("audit");
        assert_eq!(audit.status, FinalAuditStatus::Unverified);
        assert!(audit
            .unverified
            .iter()
            .any(|item| item.contains("没有任何已记录的计划任务")));
    }

    #[test]
    fn done_without_evidence_is_unverified() {
        let (_path, db, session_id) = fresh_db();
        let task = db
            .create_plan_task(&session_id, "未验证的任务", None, None, "test")
            .expect("create");
        db.update_plan_task_status(&task.id, "done", None)
            .expect("done");
        let audit = compute_final_audit(&db, &session_id, "测试目标").expect("audit");
        assert_eq!(audit.status, FinalAuditStatus::Unverified);
        assert!(audit
            .unverified
            .iter()
            .any(|item| item.contains("evidence_status=none")
                || item.contains("evidence_status=pending")));
    }

    #[test]
    fn verified_task_marks_completed() {
        let (_path, db, session_id) = fresh_db();
        let task = db
            .create_plan_task(&session_id, "完成任务", None, None, "test")
            .expect("create");
        db.update_plan_task_status(&task.id, "done", None)
            .expect("done");
        db.update_plan_task_evidence(
            &task.id,
            Some(&serde_json::json!({"verified": true})),
            "verified",
            None,
        )
        .expect("verified");
        let audit = compute_final_audit(&db, &session_id, "测试目标").expect("audit");
        assert_eq!(audit.status, FinalAuditStatus::Completed);
    }

    #[test]
    fn delivery_report_uses_checkpoint_and_run_verifications() {
        // P2-13: DeliveryReport must be assembled from real storage rows, not
        // hardcoded text or empty placeholders.
        let (_path, db, session_id) = fresh_db();
        db.create_agent_run("run-delivery", Some(&session_id), "default")
            .expect("run");
        db.create_run_plan(
            &session_id,
            "交付报告能对账真实改动和验证",
            Some("run-delivery"),
            Some(&serde_json::json!([
                "报告含真实 changedFiles 和 verification"
            ])),
            Some("最终回复能列出真实 checkpoint 与 verify 记录"),
            Some(&serde_json::json!(["不做 UI 渲染"])),
        )
        .expect("plan");
        let task = db
            .create_plan_task(
                &session_id,
                "实现 DeliveryReport",
                None,
                Some("run-delivery"),
                "test",
            )
            .expect("create");
        db.update_plan_task_status(&task.id, "done", Some("run-delivery"))
            .expect("done");
        db.update_plan_task_evidence(
            &task.id,
            Some(&serde_json::json!({"verified": true})),
            "verified",
            None,
        )
        .expect("verified");
        db.record_file_checkpoint(
            "src-tauri/src/agent/final_audit.rs",
            Some("run-delivery"),
            Some(&task.id),
            Some("before-hash"),
            Some("after-hash"),
            Some("before"),
            None,
            6,
        )
        .expect("checkpoint");
        let verification = db
            .record_task_verification(
                &task.id,
                Some("run-delivery"),
                "test",
                "cargo test final_audit",
                Some(0),
                "passed",
                "ok",
                "",
                100,
                Some(120),
            )
            .expect("verification");

        let report = compute_delivery_report(&db, &session_id, Some("run-delivery"), "fallback")
            .expect("report");

        assert_eq!(report.status, FinalAuditStatus::Completed);
        assert_eq!(report.changed_files.len(), 1);
        assert_eq!(
            report.changed_files[0].path,
            "src-tauri/src/agent/final_audit.rs"
        );
        assert_eq!(report.verification.len(), 1);
        assert_eq!(report.verification[0].id, verification.id);
        assert_eq!(report.plan.tasks[0].changed_files.len(), 1);
        assert_eq!(report.plan.tasks[0].verification_ids, vec![verification.id]);

        let rendered =
            delivery_report_text(&serde_json::to_value(&report).expect("json")).expect("render");
        for section in [
            "已完成：",
            "变更文件：",
            "已验证：",
            "未验证：",
            "风险：",
            "Mock/占位：",
        ] {
            assert!(rendered.contains(section), "missing {section}");
        }
        assert!(rendered.contains("src-tauri/src/agent/final_audit.rs"));
        assert!(rendered.contains("cargo test final_audit"));
    }

    #[test]
    fn completed_audit_without_real_verification_is_unverified_delivery() {
        // 假完成红线: verified flag alone is not enough for a completed delivery
        // report when no run_task_verifications row exists.
        let (_path, db, session_id) = fresh_db();
        db.create_agent_run("run-empty", Some(&session_id), "default")
            .expect("run");
        let task = db
            .create_plan_task(
                &session_id,
                "只有 evidence flag",
                None,
                Some("run-empty"),
                "test",
            )
            .expect("create");
        db.update_plan_task_status(&task.id, "done", Some("run-empty"))
            .expect("done");
        db.update_plan_task_evidence(
            &task.id,
            Some(&serde_json::json!({"verified": true})),
            "verified",
            None,
        )
        .expect("verified");

        let report =
            compute_delivery_report(&db, &session_id, Some("run-empty"), "目标").expect("report");

        assert_eq!(report.status, FinalAuditStatus::Unverified);
        assert!(report
            .unverified
            .iter()
            .any(|item| item.contains("run_task_verifications")));
        assert!(report.verification.is_empty());
    }

    #[test]
    fn blocked_task_marks_blocked() {
        let (_path, db, session_id) = fresh_db();
        let task = db
            .create_plan_task(&session_id, "被拦截任务", None, None, "test")
            .expect("create");
        db.update_plan_task_status(&task.id, "blocked", None)
            .expect("blocked");
        let audit = compute_final_audit(&db, &session_id, "测试目标").expect("audit");
        assert_eq!(audit.status, FinalAuditStatus::Blocked);
    }

    #[test]
    fn goal_object_from_plan_flows_into_audit() {
        // P3-4: a stored Goal (text + observableOutcome + nonGoals) overrides the
        // raw user-input fallback and surfaces in the audit.
        let (_path, db, session_id) = fresh_db();
        db.create_run_plan(
            &session_id,
            "让用户能保存并使用 API key",
            None,
            Some(&serde_json::json!(["保存后能成功调用一次"])),
            Some("用户在设置页保存 key 后，发一条消息能用该 key 成功响应"),
            Some(&serde_json::json!(["不做多 key 管理", "不做用量统计"])),
        )
        .expect("plan");

        let audit = compute_final_audit(&db, &session_id, "原始用户请求(fallback)").expect("audit");
        assert_eq!(audit.goal, "让用户能保存并使用 API key");
        assert_eq!(
            audit.observable_outcome.as_deref(),
            Some("用户在设置页保存 key 后，发一条消息能用该 key 成功响应")
        );
        assert_eq!(
            audit.non_goals,
            vec!["不做多 key 管理".to_string(), "不做用量统计".to_string()]
        );
    }

    #[test]
    fn legacy_plan_without_goal_object_keeps_fallback_and_empty_fields() {
        // 旧库/简单计划没有 observableOutcome/nonGoals：goal 仍用 plan.goal，新字段为空，不报错。
        let (_path, db, session_id) = fresh_db();
        db.create_run_plan(&session_id, "只有 goal 文本", None, None, None, None)
            .expect("plan");
        let audit = compute_final_audit(&db, &session_id, "fallback").expect("audit");
        assert_eq!(audit.goal, "只有 goal 文本");
        assert!(audit.observable_outcome.is_none());
        assert!(audit.non_goals.is_empty());
    }

    #[test]
    fn completion_guard_prefix_blocks_completion_claim() {
        // P2-3: blocked/unverified must yield a banner that negates 「已完成」;
        // completed states must NOT be intercepted.
        let blocked = completion_guard_prefix("blocked").expect("blocked banner");
        assert!(blocked.contains("尚未完成") && blocked.contains("不代表"));
        let unverified = completion_guard_prefix("unverified").expect("unverified banner");
        assert!(unverified.contains("未通过验证") && unverified.contains("已完成"));
        assert!(completion_guard_prefix("completed").is_none());
        assert!(completion_guard_prefix("completed_with_waiver").is_none());
    }
}
