use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::hooks::{self, AgentHookContext, AgentHookKind};
use crate::agent::{AgentError, ToolResult, ToolSchema};
use crate::storage::{LocalDb, PlanTaskRecord};
use crate::tools::{Tool, ToolCapability, ToolMetadata, ToolSafetyLevel};

pub const TOOL_CREATE_PLAN: &str = "create_plan";
pub const TOOL_CREATE_PLAN_TASK: &str = "create_plan_task";
pub const TOOL_UPDATE_PLAN_TASK: &str = "update_plan_task";
pub const TOOL_LIST_PLAN_TASKS: &str = "list_plan_tasks";
pub const TOOL_SET_ACTIVE_PLAN_TASK: &str = "set_active_plan_task";

pub fn is_plan_tasks_tool(name: &str) -> bool {
    matches!(
        name,
        TOOL_CREATE_PLAN
            | TOOL_CREATE_PLAN_TASK
            | TOOL_UPDATE_PLAN_TASK
            | TOOL_LIST_PLAN_TASKS
            | TOOL_SET_ACTIVE_PLAN_TASK
    )
}

fn require_session_id(session_id: &Option<String>) -> Result<String, AgentError> {
    session_id
        .clone()
        .ok_or_else(|| AgentError::Tool("当前没有绑定会话，无法记录计划。".to_string()))
}

fn plan_task_json(task: &PlanTaskRecord) -> Value {
    serde_json::to_value(task).unwrap_or_else(|_| json!({}))
}

fn required_change_reason(args: &Value) -> Result<String, AgentError> {
    args.get("change_reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AgentError::Tool("缺少 change_reason：计划变更必须说明原因。".to_string()))
}

pub struct CreatePlanTool {
    db: LocalDb,
    current_session_id: Option<String>,
}

impl CreatePlanTool {
    pub fn new(db: LocalDb, current_session_id: Option<String>) -> Self {
        Self {
            db,
            current_session_id,
        }
    }
}

#[async_trait]
impl Tool for CreatePlanTool {
    fn name(&self) -> &str {
        TOOL_CREATE_PLAN
    }

    fn description(&self) -> &str {
        "Create a run plan that captures the goal and acceptance criteria for the current session."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Create a run plan for the current session. Call this before doing any \
                non-trivial work so the session has a recorded goal, observable outcome, non-goals \
                and acceptance criteria."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "goal": {
                        "type": "string",
                        "description": "One-sentence user goal for this run."
                    },
                    "observable_outcome": {
                        "type": "string",
                        "description": "The observable RESULT that proves the goal is met — a user-visible state or behavior, NOT an action. Good: '用户能在设置页保存并成功用该 key 调用一次'. Bad: '修改设置页代码'."
                    },
                    "non_goals": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Explicitly out-of-scope items, so work isn't over-built beyond the goal."
                    },
                    "acceptance_criteria": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Concrete observable conditions that must hold for the goal to be considered met."
                    },
                    "change_reason": {
                        "type": "string",
                        "description": "Required audit reason explaining why this plan is being created or replacing the previous plan."
                    }
                },
                "required": ["goal", "change_reason"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "创建计划".to_string(),
            description_zh: "为当前会话登记目标和验收标准。".to_string(),
            capability_labels_zh: vec!["计划".to_string(), "本地数据".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::LocalData],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let session_id = require_session_id(&self.current_session_id)?;
        let goal = args
            .get("goal")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AgentError::Tool("缺少 goal 参数。".to_string()))?
            .to_string();
        let acceptance = args.get("acceptance_criteria").cloned();
        let change_reason = required_change_reason(&args)?;
        let observable_outcome = args
            .get("observable_outcome")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let non_goals = args.get("non_goals").cloned();
        let plan = self
            .db
            .create_run_plan_with_reason(
                &session_id,
                &goal,
                None,
                acceptance.as_ref(),
                observable_outcome,
                non_goals.as_ref(),
                &change_reason,
                "agent",
            )
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(ToolResult::success(
            "已登记计划目标。下一步：用 create_plan_task 拆分子任务，并用 set_active_plan_task 激活。",
            json!({ "plan": plan }),
        ))
    }
}

pub struct CreatePlanTaskTool {
    db: LocalDb,
    current_session_id: Option<String>,
}

impl CreatePlanTaskTool {
    pub fn new(db: LocalDb, current_session_id: Option<String>) -> Self {
        Self {
            db,
            current_session_id,
        }
    }
}

#[async_trait]
impl Tool for CreatePlanTaskTool {
    fn name(&self) -> &str {
        TOOL_CREATE_PLAN_TASK
    }

    fn description(&self) -> &str {
        "Create a single plan task with acceptance criteria and verification spec."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Create one plan task. Each task should have at least one acceptance \
                criterion and ideally one verify entry (command or check) before it can be done."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Short imperative title (max 240 chars)."
                    },
                    "parent_id": {
                        "type": ["string", "null"],
                        "description": "Optional parent task id for subtasks."
                    },
                    "acceptance_criteria": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Observable conditions that must hold when this task is done."
                    },
                    "verify": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "kind": { "type": "string" },
                                "command": { "type": "string" }
                            }
                        },
                        "description": "Verification entries ({ kind, command, required? }); kind may be any stable audit label such as frontend_build."
                    },
                    "source": {
                        "type": "string",
                        "description": "Source label (default 'agent')."
                    },
                    "change_reason": {
                        "type": "string",
                        "description": "Required audit reason explaining why this task is being added to the plan."
                    }
                },
                "required": ["title", "change_reason"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "新增计划任务".to_string(),
            description_zh: "把计划拆成一个具体可验证的子任务。".to_string(),
            capability_labels_zh: vec!["计划".to_string(), "本地数据".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::LocalData],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let session_id = require_session_id(&self.current_session_id)?;
        let title = args
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AgentError::Tool("缺少 title 参数。".to_string()))?
            .to_string();
        let parent_id = args
            .get("parent_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let source = args
            .get("source")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("agent");
        let acceptance = args.get("acceptance_criteria").cloned();
        let verify = args.get("verify").cloned();
        let change_reason = required_change_reason(&args)?;
        let task = self
            .db
            .create_plan_task_full_with_reason(
                &session_id,
                &title,
                parent_id,
                None,
                source,
                acceptance.as_ref(),
                verify.as_ref(),
                &change_reason,
                "agent",
            )
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(ToolResult::success(
            "已新增计划任务。要写文件或跑命令时先用 set_active_plan_task 激活该任务。",
            json!({ "task": plan_task_json(&task) }),
        ))
    }
}

pub struct UpdatePlanTaskTool {
    db: LocalDb,
    current_session_id: Option<String>,
    /// T24: project root for auto-running `task.verify[0]` when status → done.
    project_root: Option<std::path::PathBuf>,
}

impl UpdatePlanTaskTool {
    pub fn new(db: LocalDb, current_session_id: Option<String>) -> Self {
        Self {
            db,
            current_session_id,
            project_root: None,
        }
    }

    pub fn with_project_root(mut self, root: Option<std::path::PathBuf>) -> Self {
        self.project_root = root;
        self
    }
}

#[async_trait]
impl Tool for UpdatePlanTaskTool {
    fn name(&self) -> &str {
        TOOL_UPDATE_PLAN_TASK
    }

    fn description(&self) -> &str {
        "Update a plan task status or evidence. Marking done without evidence flags it pending."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Update a plan task. Set status to one of: pending, doing, verifying, \
                done, blocked, waived, cancelled, skipped. When marking 'done' with auto-verify, \
                the task only stays done after required verification passes; failed required \
                verification keeps it verifying with evidence_status=failed."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "status": {
                        "type": "string",
                        "enum": [
                            "pending", "doing", "verifying", "done",
                            "blocked", "waived", "cancelled", "skipped"
                        ]
                    },
                    "evidence": {
                        "description": "Free-form evidence payload (commands run, file paths, verification ids).",
                        "type": ["object", "array", "null"]
                    },
                    "evidence_status": {
                        "type": "string",
                        "enum": ["none", "pending", "verified", "waived", "failed"]
                    },
                    "blocked_reason": { "type": ["string", "null"] },
                    "change_reason": {
                        "type": "string",
                        "description": "Required audit reason explaining why this task status/evidence is being changed."
                    }
                },
                "required": ["task_id", "change_reason"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "更新计划任务".to_string(),
            description_zh: "更新任务状态、证据或拦截原因。".to_string(),
            capability_labels_zh: vec!["计划".to_string(), "本地数据".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::LocalData],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let _session_id = require_session_id(&self.current_session_id)?;
        let task_id = args
            .get("task_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AgentError::Tool("缺少 task_id 参数。".to_string()))?
            .to_string();
        let raw_status = args.get("status").and_then(Value::as_str);
        let evidence = args.get("evidence").cloned();
        let change_reason = required_change_reason(&args)?;
        let mut explicit_status = args
            .get("evidence_status")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let blocked_reason = args.get("blocked_reason").and_then(Value::as_str);

        if let Some(status) = raw_status {
            // T7.2 — hard gate: refuse to mark `done` without real evidence or an
            // explicit verified/waived evidence_status. Soft-pending was leaking
            // tasks into "done" with no proof; plan §M3.7 wants this blocked.
            if status.eq_ignore_ascii_case("done") {
                let has_real_evidence = evidence
                    .as_ref()
                    .map(|v| match v {
                        Value::Null => false,
                        Value::Object(map) => !map.is_empty(),
                        Value::Array(arr) => !arr.is_empty(),
                        Value::String(s) => !s.trim().is_empty(),
                        _ => true,
                    })
                    .unwrap_or(false);
                let waived_or_verified = explicit_status
                    .as_deref()
                    .map(|s| matches!(s, "verified" | "waived"))
                    .unwrap_or(false);
                if !has_real_evidence && !waived_or_verified {
                    let mut result = ToolResult::recoverable_error(
                        "无法将任务标记为 done：缺少 evidence 且未声明 verified/waived 证据状态。",
                        vec![
                            "先通过 run_verify 或具体工具产生证据，再回写到 update_plan_task 的 evidence。".to_string(),
                            "如果不需要验证，先把 evidence_status 设为 waived 再调 done。".to_string(),
                            "或者把状态改成 verifying 表示证据正在收集中。".to_string(),
                        ],
                    );
                    result.data = json!({
                        "taskId": task_id,
                        "reason": "missing_evidence",
                        "expected": "提供 evidence 字段（命令记录 / 文件路径 / 截图等），或显式将 evidence_status 设为 verified/waived。"
                    });
                    return Ok(result);
                }

                // Patch 17 / #18: BeforeTaskDone hook gate.
                // If any hook in the registry matches and returns Block, the done
                // transition is refused and the task is flipped to blocked instead.
                let task_for_hook = self
                    .db
                    .get_plan_task(&task_id)
                    .map_err(|e| AgentError::Tool(e.to_string()))?;
                let title_for_hook = task_for_hook.title.clone();
                let hook_ctx =
                    AgentHookContext::new(AgentHookKind::BeforeTaskDone, task_id.clone())
                        .with_session(Some(task_for_hook.session_id.clone()))
                        .with_task(Some(task_id.clone()))
                        .with_extra("task_title", &title_for_hook);
                let hook_runs = hooks::dispatch_global(&hook_ctx, Some(&title_for_hook)).await;
                if let Some(blocker) = hook_runs.iter().find(|r| r.outcome.is_block()) {
                    self.db
                        .update_plan_task_status_with_reason(
                            &task_id,
                            "blocked",
                            None,
                            &change_reason,
                            "agent",
                        )
                        .map_err(|e| AgentError::Tool(e.to_string()))?;
                    let reason = match &blocker.outcome {
                        crate::agent::hooks::HookOutcome::Block { reason } => reason.clone(),
                        _ => "hook blocked".to_string(),
                    };
                    let mut result = ToolResult::recoverable_error(
                        format!("BeforeTaskDone hook 阻塞了 done 转换：{reason}"),
                        vec![
                            "查看 hookRuns 里的 stderr_tail / stdout_tail 调查原因。".to_string(),
                            "修好后重新调 update_plan_task status=done。".to_string(),
                        ],
                    );
                    result.data = json!({
                        "taskId": task_id,
                        "reason": "before_task_done_hook_blocked",
                        "hookRuns": hook_runs,
                    });
                    return Ok(result);
                }
            }
            let auto_verify_done = status.eq_ignore_ascii_case("done") && explicit_status.is_none();
            let status_to_store = if auto_verify_done {
                // P2-2 fix: do not expose status=done while required auto-verify
                // is still running or has failed. A passing verify writes done
                // below; a failing one stays verifying with evidence_status=failed.
                "verifying"
            } else {
                status
            };
            self.db
                .update_plan_task_status_with_reason(
                    &task_id,
                    status_to_store,
                    None,
                    &change_reason,
                    "agent",
                )
                .map_err(|e| AgentError::Tool(e.to_string()))?;
            if auto_verify_done {
                // T24: auto-trigger verify before defaulting to pending.
                let task_after_status = self
                    .db
                    .get_plan_task(&task_id)
                    .map_err(|e| AgentError::Tool(e.to_string()))?;
                match crate::tools::run_verify::auto_verify_done_task(
                    &self.db,
                    &task_after_status,
                    self.project_root.as_deref(),
                )
                .await
                {
                    Some(outcome) => {
                        // T25: preserve the blocked_reason that auto_verify wrote.
                        // Return early — the auto helper already wrote the final
                        // evidence row; running update_plan_task_evidence again
                        // below would clobber blocked_reason.
                        if outcome.passed {
                            self.db
                                .update_plan_task_status_with_reason(
                                    &task_id,
                                    "done",
                                    None,
                                    &change_reason,
                                    "agent",
                                )
                                .map_err(|e| AgentError::Tool(e.to_string()))?;
                        }
                        let task = self
                            .db
                            .get_plan_task(&task_id)
                            .map_err(|e| AgentError::Tool(e.to_string()))?;
                        let summary = if outcome.blocked {
                            let reason = outcome
                                .blocked_reason
                                .as_deref()
                                .unwrap_or("auto-verify 连续失败");
                            format!("auto-verify 连续失败，任务已自动 blocked：{reason}")
                        } else if outcome.passed {
                            "任务已更新并通过 auto-verify。".to_string()
                        } else {
                            "auto-verify 失败，任务保持 verifying，evidence_status=failed。"
                                .to_string()
                        };
                        return Ok(ToolResult::success(summary, json!({ "task": task })));
                    }
                    None => {
                        self.db
                            .update_plan_task_status_with_reason(
                                &task_id,
                                "done",
                                None,
                                &change_reason,
                                "agent",
                            )
                            .map_err(|e| AgentError::Tool(e.to_string()))?;
                        explicit_status = Some("pending".to_string());
                    }
                }
            }
            if status.eq_ignore_ascii_case("blocked") && explicit_status.is_none() {
                explicit_status = Some("failed".to_string());
            }
        }

        let evidence_status = explicit_status.unwrap_or_else(|| "none".to_string());
        let task = self
            .db
            .update_plan_task_evidence_with_reason(
                &task_id,
                evidence.as_ref(),
                &evidence_status,
                blocked_reason,
                &change_reason,
                "agent",
            )
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        let summary = if task.evidence_status == "pending" {
            "任务已更新，但证据状态为 pending，需要验证通过才能算完成。".to_string()
        } else if task.evidence_status == "verified" {
            "任务已更新并标记为验证通过。".to_string()
        } else {
            "任务已更新。".to_string()
        };
        Ok(ToolResult::success(
            summary,
            json!({ "task": plan_task_json(&task) }),
        ))
    }
}

pub struct ListPlanTasksTool {
    db: LocalDb,
    current_session_id: Option<String>,
}

impl ListPlanTasksTool {
    pub fn new(db: LocalDb, current_session_id: Option<String>) -> Self {
        Self {
            db,
            current_session_id,
        }
    }
}

#[async_trait]
impl Tool for ListPlanTasksTool {
    fn name(&self) -> &str {
        TOOL_LIST_PLAN_TASKS
    }

    fn description(&self) -> &str {
        "List plan tasks for the current session."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "List the current session's plan tasks with status, evidence_status, \
                active flag and blocked_reason."
                .to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "查看计划任务".to_string(),
            description_zh: "读取当前会话登记的计划任务列表。".to_string(),
            capability_labels_zh: vec!["只读".to_string(), "本地数据".to_string()],
            safety_label_zh: "安全".to_string(),
            capabilities: vec![ToolCapability::ReadOnly, ToolCapability::LocalData],
            safety_level: ToolSafetyLevel::Safe,
            mutates_state: false,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, _args: Value) -> Result<ToolResult, AgentError> {
        let session_id = require_session_id(&self.current_session_id)?;
        let tasks = self
            .db
            .list_plan_tasks(&session_id)
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        let active = tasks.iter().find(|t| t.active).map(|t| t.id.clone());
        Ok(ToolResult::success(
            format!("当前会话有 {} 个计划任务。", tasks.len()),
            json!({
                "tasks": tasks.iter().map(plan_task_json).collect::<Vec<_>>(),
                "active_task_id": active,
            }),
        ))
    }
}

pub struct SetActivePlanTaskTool {
    db: LocalDb,
    current_session_id: Option<String>,
}

impl SetActivePlanTaskTool {
    pub fn new(db: LocalDb, current_session_id: Option<String>) -> Self {
        Self {
            db,
            current_session_id,
        }
    }
}

#[async_trait]
impl Tool for SetActivePlanTaskTool {
    fn name(&self) -> &str {
        TOOL_SET_ACTIVE_PLAN_TASK
    }

    fn description(&self) -> &str {
        "Activate a plan task (session-scoped). Pass null to clear the active task."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: "Set the active plan task for the current session. While an active task \
                exists, mutating tools (write_file/edit_file/create_directory/run_command/\
                invoke_mcp_tool) record their effects against that task. With no active task, \
                those tools are blocked."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": ["string", "null"],
                        "description": "Task id to activate; null clears the active task."
                    },
                    "change_reason": {
                        "type": "string",
                        "description": "Required audit reason explaining why the active task is changing."
                    }
                },
                "required": ["change_reason"]
            }),
        }
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name().to_string(),
            description: self.description().to_string(),
            label_zh: "切换活跃任务".to_string(),
            description_zh: "选定当前会话要执行的活跃任务，写入和命令会被这个任务约束。"
                .to_string(),
            capability_labels_zh: vec!["计划".to_string(), "本地数据".to_string()],
            safety_label_zh: "敏感".to_string(),
            capabilities: vec![ToolCapability::LocalData],
            safety_level: ToolSafetyLevel::Sensitive,
            mutates_state: true,
            requires_confirmation: false,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, AgentError> {
        let session_id = require_session_id(&self.current_session_id)?;
        let task_id = args
            .get("task_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let change_reason = required_change_reason(&args)?;
        let active = self
            .db
            .set_active_plan_task_with_reason(&session_id, task_id, &change_reason, "agent")
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        let summary = match (&active, task_id) {
            (Some(task), _) => format!("已激活任务：{}", task.title),
            (None, None) => "已清除当前活跃任务。".to_string(),
            (None, Some(_)) => "未找到该任务。".to_string(),
        };
        Ok(ToolResult::success(
            summary,
            json!({
                "active": active.as_ref().map(plan_task_json),
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::LocalDb;
    use serde_json::json;
    use uuid::Uuid;

    fn setup_db() -> (std::path::PathBuf, LocalDb, String) {
        let path = std::env::temp_dir().join(format!("aura_plan_tasks_{}.db", Uuid::new_v4()));
        let db = LocalDb::open(path.clone()).expect("open db");
        let session = db.create_session("plan-tasks-test").expect("session");
        (path, db, session.id)
    }

    #[tokio::test]
    async fn create_and_activate_task_round_trip() {
        let (_path, db, session_id) = setup_db();
        let create_task = CreatePlanTaskTool::new(db.clone(), Some(session_id.clone()));
        let result = create_task
            .execute(json!({
                "title": "重构 plan_tasks 模块",
                "change_reason": "测试创建任务"
            }))
            .await
            .expect("create task");
        let task = result.data["task"].clone();
        let task_id = task["id"].as_str().expect("id").to_string();

        let activate = SetActivePlanTaskTool::new(db.clone(), Some(session_id.clone()));
        let active_result = activate
            .execute(json!({
                "task_id": task_id.clone(),
                "change_reason": "测试激活任务"
            }))
            .await
            .expect("activate");
        assert!(active_result.data["active"]["active"]
            .as_bool()
            .unwrap_or(false));

        let list = ListPlanTasksTool::new(db.clone(), Some(session_id.clone()));
        let list_result = list.execute(json!({})).await.expect("list");
        assert_eq!(
            list_result.data["active_task_id"]
                .as_str()
                .map(str::to_string),
            Some(task_id.clone())
        );

        // Clear active
        let clear = activate
            .execute(json!({ "change_reason": "测试清除活跃任务" }))
            .await
            .expect("clear");
        assert!(clear.data["active"].is_null());
    }

    #[tokio::test]
    async fn update_done_without_evidence_is_rejected_recoverable() {
        // T7.2 — marking a task done with no evidence and no explicit
        // verified/waived flag must return a recoverable error, not soft-pending.
        let (_path, db, session_id) = setup_db();
        let task = db
            .create_plan_task(&session_id, "测试任务", None, None, "test")
            .expect("create");
        let update = UpdatePlanTaskTool::new(db.clone(), Some(session_id.clone()));
        let result = update
            .execute(json!({
                "task_id": task.id,
                "status": "done",
                "change_reason": "测试无证据完成"
            }))
            .await
            .expect("returns a ToolResult");
        assert!(
            matches!(result.status, crate::agent::ToolResultStatus::Error),
            "expected Error status, got {:?}",
            result.status
        );
        assert!(result.recoverable, "expected recoverable=true");
        assert_eq!(result.data["reason"].as_str(), Some("missing_evidence"));

        // Task status should still be pending (not flipped to done).
        let fetched = db.get_plan_task(&task.id).expect("fetch");
        assert_ne!(fetched.status, "done");
    }

    #[tokio::test]
    async fn update_done_with_evidence_succeeds() {
        // T7.2 — when real evidence is supplied, done is accepted and
        // evidence_status auto-defaults to pending (awaiting verifier).
        let (_path, db, session_id) = setup_db();
        let task = db
            .create_plan_task(&session_id, "带证据的任务", None, None, "test")
            .expect("create");
        let update = UpdatePlanTaskTool::new(db.clone(), Some(session_id.clone()));
        let result = update
            .execute(json!({
                "task_id": task.id,
                "status": "done",
                "evidence": { "kind": "test", "details": "passes" },
                "change_reason": "测试带证据完成"
            }))
            .await
            .expect("returns ok");
        assert!(matches!(
            result.status,
            crate::agent::ToolResultStatus::Success
        ));
        let evidence_status = result.data["task"]["evidenceStatus"]
            .as_str()
            .expect("evidenceStatus");
        assert_eq!(evidence_status, "pending");
    }

    #[tokio::test]
    async fn auto_verify_passes_flips_evidence_to_verified() {
        // T24: when task.verify[0] succeeds, evidence_status should auto-flip
        // to `verified` without needing a separate run_verify call.
        let (_path, db, session_id) = setup_db();
        let echo_cmd = if cfg!(windows) {
            "cmd /C exit 0"
        } else {
            "true"
        };
        let task = db
            .create_plan_task_full(
                &session_id,
                "auto verify task",
                None,
                None,
                "test",
                None,
                Some(&serde_json::json!([echo_cmd])),
            )
            .expect("create");
        let update = UpdatePlanTaskTool::new(db.clone(), Some(session_id.clone()));
        let result = update
            .execute(json!({
                "task_id": task.id,
                "status": "done",
                "evidence": { "kind": "test", "details": "ok" },
                "change_reason": "测试自动验证通过"
            }))
            .await
            .expect("ok");
        assert!(matches!(
            result.status,
            crate::agent::ToolResultStatus::Success
        ));
        assert_eq!(
            result.data["task"]["evidenceStatus"].as_str(),
            Some("verified"),
        );
        assert_eq!(result.data["task"]["status"].as_str(), Some("done"));
    }

    #[tokio::test]
    async fn auto_verify_fail_marks_failed() {
        // T24+T25: a failing verify command marks evidence_status=failed and
        // must not leave the task in status=done.
        let (_path, db, session_id) = setup_db();
        let fail_cmd = if cfg!(windows) {
            "cmd /C exit 1"
        } else {
            "false"
        };
        let task = db
            .create_plan_task_full(
                &session_id,
                "auto verify fails",
                None,
                None,
                "test",
                None,
                Some(&serde_json::json!([fail_cmd])),
            )
            .expect("create");
        let update = UpdatePlanTaskTool::new(db.clone(), Some(session_id.clone()));
        let result = update
            .execute(json!({
                "task_id": task.id,
                "status": "done",
                "evidence": { "kind": "test", "details": "tried" },
                "change_reason": "测试自动验证失败"
            }))
            .await
            .expect("ok");
        assert_eq!(
            result.data["task"]["evidenceStatus"].as_str(),
            Some("failed"),
        );
        assert_eq!(result.data["task"]["status"].as_str(), Some("verifying"));
    }

    #[tokio::test]
    async fn repair_loop_blocks_after_three_same_signature_failures() {
        // T25: 3 consecutive failures with the same stderr signature must
        // flip the task to blocked (evidence_status=failed, status=blocked).
        let (_path, db, session_id) = setup_db();
        let fail_cmd = if cfg!(windows) {
            "cmd /C exit 1"
        } else {
            "false"
        };
        let task = db
            .create_plan_task_full(
                &session_id,
                "repair loop task",
                None,
                None,
                "test",
                None,
                Some(&serde_json::json!([fail_cmd])),
            )
            .expect("create");
        let update = UpdatePlanTaskTool::new(db.clone(), Some(session_id.clone()));
        let mut last_status: Option<String> = None;
        for _ in 0..3 {
            // Reset status back to "pending" so we can re-trigger done.
            db.update_plan_task_status(&task.id, "pending", None)
                .expect("reset");
            let result = update
                .execute(json!({
                    "task_id": task.id,
                    "status": "done",
                    "evidence": { "kind": "test", "details": "x" },
                    "change_reason": "测试修复循环"
                }))
                .await
                .expect("ok");
            last_status = result.data["task"]["status"].as_str().map(str::to_string);
        }
        // After the third failure, the auto-block kicks in.
        assert_eq!(last_status.as_deref(), Some("blocked"));
        let fetched = db.get_plan_task(&task.id).expect("fetch");
        assert_eq!(fetched.evidence_status, "failed");
        assert!(fetched
            .blocked_reason
            .as_deref()
            .map(|s| s.contains("auto"))
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn missing_session_returns_tool_error() {
        let path = std::env::temp_dir().join(format!("aura_plan_tasks_{}.db", Uuid::new_v4()));
        let db = LocalDb::open(path).expect("open db");
        let tool = CreatePlanTaskTool::new(db, None);
        let err = tool
            .execute(json!({ "title": "x", "change_reason": "测试缺会话" }))
            .await
            .expect_err("session missing");
        assert!(matches!(err, AgentError::Tool(_)));
    }
}
