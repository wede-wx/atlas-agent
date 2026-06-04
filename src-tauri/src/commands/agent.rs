use tauri::{Emitter, State, Window};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent::{
    abort_queued_graph_run, append_external_task_stream_event, builtin_eval_suite,
    builtin_eval_suites, cancel_external_task, classify_intent, create_external_task_mapping,
    enqueue_graph_run, estimate_cost_from_parts,
    estimate_model_text_cost as estimate_model_text_cost_inner,
    evaluate_installed_plugin_quality_gate, evaluate_trajectory_completion, export_run_trajectory,
    get_agent_profile, get_external_task_mapping, get_graph_queue_control, graph_node_traces,
    infer_project_root, ingest_connector_knowledge, inspect_code_intelligence,
    list_agent_profile_metadata, list_external_task_stream_events, list_graph_queue,
    list_model_quality_events, list_model_route_decisions, load_agent_rule_context,
    load_skill_snapshot, parse_skill_markdown, parse_skill_states, persist_eval_report,
    plugin_eval_registry_entry, prepare_lsp_session, project_skills_dir,
    protocol_compatibility_matrix, recall_knowledge, recall_knowledge_with_feedback,
    record_model_quality_event as record_model_quality_event_inner,
    record_model_route_decision as record_model_route_decision_inner, replay_trajectory_readonly,
    route_economics_decision, run_audit_feed, run_eval_suite_verifiers, run_progress_summary,
    run_terminal_feed, sanitize_skill_name, scan_project_snapshot, score_eval_suite,
    select_model_route, set_graph_queue_paused, skill_version_registry, status_semantic,
    team_preset_permission_report, update_external_task_lifecycle,
    user_message_should_use_conversation_history, user_skills_dir,
    validate_workspace_cwd as validate_workspace_cwd_for_record, write_memory_from_event, Agent,
    AgentAttachment, AgentError, AgentEvent, AgentGuidanceMessage, AgentProfile,
    AgentProfileMetadata, AgentRunEvent, AgentToolAuditEvent, CodeIntelligenceReport,
    CodeIntelligenceRequest, CreateAgentGraphSpec, CreateTeamRunSpec, DurableAgentGraphRuntime,
    DurableTeamRuntime, EvalCaseOutcome, EvalRunOptions, EvalRunReport, EvalSuite, EvalSuiteReport,
    ExternalTask, FallbackClientEntry, FallbackLLMClient, HandoffContract,
    KnowledgeConnectorIngestReport, KnowledgeConnectorIngestRequest, KnowledgeRecallRequest,
    LLMClient, LspSessionPlan, LspSessionSpec, MemoryWriteEvent, Message, ModelCostEstimate,
    ModelQualityEvent, ModelRouteDecision, ModelRouteDecisionAudit, ModelRoutePolicy,
    ModelRouteRequest, ModelTextCostEstimate, PluginEvalRegistryEntry, PluginQualityGate,
    PluginQualityGateRequest, ProjectSnapshot, ProtocolCompatibilityEntry, ProtocolLifecycleUpdate,
    ProtocolRunMapping, ProtocolStreamEvent, QueuedGraphRun, RecordModelQualityEventRequest,
    ReplayReport, RetrievalContext, Role, RunAuditFeed, RunProgressSummary, RunTerminalFeed,
    SkillMetadata, SkillRegistry, SkillRegistrySnapshot, SkillState, SkillVersionRecord,
    StatusSemantic, TeamExecutionOptions, TeamExecutionPlan, TeamPresetPermissionReport,
    TokenBudget, TokenBudgetCircuitBreaker, TokenBudgetHardLimitAction, TokenBudgetScope,
    TokenBudgetSnapshot, TrajectoryEvalReport, TrajectoryExport, TrajectoryExportOptions,
    UserIntent, WorkflowQueueControl, WorkflowTraceReport, WorkspaceCwdVerdict,
    WorkspaceGitHookInstallReport, WorkspaceGitHookSpec, WorkspaceLifecycleRuntime,
    WorkspaceLifecycleSpec, WorkspaceSetupRunOptions, AGENT_SKILL_STATE_KEY,
};
use crate::config::{Config, ModelConnectionConfig};
use crate::storage::{
    AddKnowledgeItemPayload, AgentGraphSnapshot, AgentRunRecord, AgentRunStepRecord,
    AgentToolAuditRecord, BrowserAgentStepRecord, HandoffRequestRecord, KnowledgeItemRecord,
    LocalDb, LogAgentToolAuditPayload, LogModelUsagePayload, LogPermissionDecisionPayload,
    MessageRecord, ModelUsageSummary, PermissionDecisionRecord, RecordArtifactPayload, RunTimeline,
    SaveMessagePayload, TeamMessageRecord, TeamRunSnapshot, WorkspaceLifecycleRecord,
    WorkspaceLifecycleSnapshot,
};
use crate::tools::checkpoint::{build_run_diff, RunDiff};
use crate::tools::{AgentPermissionMode, SubAgentRole, ToolAccessPolicy};
use crate::{create_llm_client, create_runtime_tool_registry, AppState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const AGENT_TOKEN_BUDGET_POLICY_STATE_KEY: &str = "agent_token_budget_policy";
const AGENT_MODEL_ROUTING_POLICY_STATE_KEY: &str = "agent_model_routing_policy";
const DEFAULT_RUN_SOFT_LIMIT_TOKENS: i64 = 120_000;
const DEFAULT_RUN_HARD_LIMIT_TOKENS: i64 = 160_000;
const DEFAULT_SESSION_SOFT_LIMIT_TOKENS: i64 = 300_000;
const DEFAULT_SESSION_HARD_LIMIT_TOKENS: i64 = 450_000;
const DEFAULT_DAY_SOFT_LIMIT_TOKENS: i64 = 750_000;
const DEFAULT_DAY_HARD_LIMIT_TOKENS: i64 = 1_000_000;
const DEFAULT_BREAKER_HIGH_TOTAL_TOKENS: i64 = 40_000;
const DEFAULT_BREAKER_LOW_OUTPUT_TOKENS: i64 = 64;
const DEFAULT_BREAKER_CONSECUTIVE_LOW_YIELD_TRIGGER: i64 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentTokenBudgetPolicy {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_run_soft_limit_tokens")]
    run_soft_limit_tokens: i64,
    #[serde(default = "default_run_hard_limit_tokens")]
    run_hard_limit_tokens: i64,
    #[serde(default = "default_session_soft_limit_tokens")]
    session_soft_limit_tokens: i64,
    #[serde(default = "default_session_hard_limit_tokens")]
    session_hard_limit_tokens: i64,
    #[serde(default = "default_day_soft_limit_tokens")]
    day_soft_limit_tokens: i64,
    #[serde(default = "default_day_hard_limit_tokens")]
    day_hard_limit_tokens: i64,
    #[serde(default)]
    on_hard_limit: TokenBudgetHardLimitAction,
    #[serde(default = "default_true")]
    circuit_breaker_enabled: bool,
    #[serde(default = "default_breaker_high_total_tokens")]
    breaker_high_total_tokens: i64,
    #[serde(default = "default_breaker_low_output_tokens")]
    breaker_low_output_tokens: i64,
    #[serde(default = "default_breaker_consecutive_low_yield_trigger")]
    breaker_consecutive_low_yield_trigger: i64,
}

impl Default for AgentTokenBudgetPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            run_soft_limit_tokens: DEFAULT_RUN_SOFT_LIMIT_TOKENS,
            run_hard_limit_tokens: DEFAULT_RUN_HARD_LIMIT_TOKENS,
            session_soft_limit_tokens: DEFAULT_SESSION_SOFT_LIMIT_TOKENS,
            session_hard_limit_tokens: DEFAULT_SESSION_HARD_LIMIT_TOKENS,
            day_soft_limit_tokens: DEFAULT_DAY_SOFT_LIMIT_TOKENS,
            day_hard_limit_tokens: DEFAULT_DAY_HARD_LIMIT_TOKENS,
            on_hard_limit: TokenBudgetHardLimitAction::PauseAndConfirm,
            circuit_breaker_enabled: true,
            breaker_high_total_tokens: DEFAULT_BREAKER_HIGH_TOTAL_TOKENS,
            breaker_low_output_tokens: DEFAULT_BREAKER_LOW_OUTPUT_TOKENS,
            breaker_consecutive_low_yield_trigger: DEFAULT_BREAKER_CONSECUTIVE_LOW_YIELD_TRIGGER,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_run_soft_limit_tokens() -> i64 {
    DEFAULT_RUN_SOFT_LIMIT_TOKENS
}

fn default_run_hard_limit_tokens() -> i64 {
    DEFAULT_RUN_HARD_LIMIT_TOKENS
}

fn default_session_soft_limit_tokens() -> i64 {
    DEFAULT_SESSION_SOFT_LIMIT_TOKENS
}

fn default_session_hard_limit_tokens() -> i64 {
    DEFAULT_SESSION_HARD_LIMIT_TOKENS
}

fn default_day_soft_limit_tokens() -> i64 {
    DEFAULT_DAY_SOFT_LIMIT_TOKENS
}

fn default_day_hard_limit_tokens() -> i64 {
    DEFAULT_DAY_HARD_LIMIT_TOKENS
}

fn default_breaker_high_total_tokens() -> i64 {
    DEFAULT_BREAKER_HIGH_TOTAL_TOKENS
}

fn default_breaker_low_output_tokens() -> i64 {
    DEFAULT_BREAKER_LOW_OUTPUT_TOKENS
}

fn default_breaker_consecutive_low_yield_trigger() -> i64 {
    DEFAULT_BREAKER_CONSECUTIVE_LOW_YIELD_TRIGGER
}

fn build_subagent_instruction(profile: &AgentProfile, agent_mode: &str) -> String {
    if agent_mode == "code_review" {
        return format!(
            "你现在以代码审查子代理「{}」身份执行 Aura `/代码审查` 只读审查。子代理不能绕过当前会话权限，不能替主 Agent 宣布最终完成。\n\n[代码审查命令硬性边界]\nAURA_CODE_REVIEW_COMMAND.md 和本段边界优先于子代理定义中任何要求运行本地命令、修改文件、提交、发布评论或输出执行计划的内容。只允许基于已提供文本和可读取文件做审查；缺少 diff、测试、CI 或真实验证证据时，按规则报告 `[信息缺失]` 或未验证风险，不要声称已经完成本地检查。\n\n[子代理摘要]\nSource: {}\nDescription: {}\nModel hint: {}\nAllowed review capabilities: Read, Grep, Glob\n\n输出 findings first，只给可被主 Agent 汇总的结果、风险和建议。",
            profile.metadata.name,
            profile.metadata.source,
            profile.metadata.description,
            if profile.metadata.model.trim().is_empty() {
                "default"
            } else {
                profile.metadata.model.as_str()
            }
        );
    }

    format!(
        "你现在以子代理「{}」身份执行一个受控子任务。子代理不能绕过当前会话权限，不能替主 Agent 宣布最终完成；你只输出本子任务的分析、执行结果、风险和建议。\n\n[子代理定义]\nSource: {}\nDescription: {}\nModel hint: {}\nDeclared tools: {}\n\n{}",
        profile.metadata.name,
        profile.metadata.source,
        profile.metadata.description,
        if profile.metadata.model.trim().is_empty() {
            "default"
        } else {
            profile.metadata.model.as_str()
        },
        if profile.metadata.tools.is_empty() {
            "none".to_string()
        } else {
            profile.metadata.tools.join(", ")
        },
        profile.instructions
    )
}

const MEMORY_INJECTION_STATE_KEY: &str = "agent_memory_injection_enabled";
const AGENT_SKILL_AUTO_MODE_STATE_KEY: &str = "agent_skill_auto_mode";
const ATLAS_DELIVERY_REPORT_IN_CHAT_STATE_KEY: &str = "atlas_delivery_report_in_chat";
const ATLAS_CONTEXT_TOKEN_LIMIT: usize = 300_000;
const ATLAS_CONTEXT_COMPRESSION_TRIGGER_TOKENS: usize = 270_000;
const CONTEXT_TOKEN_ESTIMATE_CHARS_PER_TOKEN: usize = 4;
const CONTEXT_WINDOW_RECENT_MESSAGE_LIMIT: usize = 10_000;
const CONTEXT_WINDOW_SUMMARY_TRIGGER_MESSAGE_COUNT: usize = 10_000;
const CONTEXT_WINDOW_PROTECTED_RECENT_MESSAGE_FLOOR: usize = 12;
const CONTEXT_WINDOW_SOFT_CHAR_BUDGET: usize =
    ATLAS_CONTEXT_COMPRESSION_TRIGGER_TOKENS * CONTEXT_TOKEN_ESTIMATE_CHARS_PER_TOKEN;
const CONTEXT_WINDOW_AUDIT_NOTE_RESERVED_CHARS: usize = 1_200;

fn memory_injection_enabled(db: &LocalDb) -> bool {
    db.get_app_state(MEMORY_INJECTION_STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

fn should_append_delivery_report_to_chat(db: &LocalDb) -> bool {
    db.get_app_state(ATLAS_DELIVERY_REPORT_IN_CHAT_STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

#[tauri::command]
pub async fn agent_chat(
    message: String,
    history: Option<Vec<Message>>,
    state: State<'_, AppState>,
    window: Window,
) -> Result<String, String> {
    let history = build_backend_history(history.unwrap_or_default(), &state).await;
    run_agent(
        message,
        history,
        state,
        window,
        None,
        "chat",
        Vec::new(),
        None,
    )
    .await
}

#[tauri::command]
pub async fn agent_chat_v2(
    session_id: String,
    message: String,
    display_message: Option<String>,
    mode: Option<String>,
    attachments: Option<Vec<AgentAttachment>>,
    state: State<'_, AppState>,
    window: Window,
) -> Result<String, String> {
    let agent_mode = normalize_agent_mode(mode);
    let attachments = attachments.unwrap_or_default();
    let active_run_id = active_run_for_session(&state, &session_id).await;
    if active_run_id.is_none() {
        ensure_context_window_compressed_for_send(
            &state.local_db,
            &session_id,
            &agent_mode,
            &message,
            attachments.clone(),
        )?;
    }
    let history = if active_run_id.is_none() {
        build_conversation_context_for_current_message(&session_id, &state, &agent_mode, &message)?
    } else {
        Vec::new()
    };
    let persisted_message = display_message.unwrap_or_else(|| message.clone());
    let uses_internal_prompt = persisted_message != message;

    state
        .local_db
        .save_message(
            &session_id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: persisted_message.clone(),
                created_at: None,
                metadata: serde_json::json!({
                    "source": "agent_chat_v2",
                    "mode": agent_mode,
                    "usesInternalPrompt": uses_internal_prompt,
                    "attachments": attachments.clone()
                }),
            },
        )
        .map_err(|e| e.to_string())?;

    if let Some(run_id) = active_run_id {
        queue_run_guidance(&state, &run_id, message.clone(), attachments.clone()).await;
        state
            .local_db
            .append_agent_run_step(
                &run_id,
                "guidance",
                "finished",
                "收到用户补充消息，已排队等待当前任务处理。",
                serde_json::json!({
                    "sessionId": session_id,
                    "content": persisted_message,
                    "attachments": attachments.len()
                }),
                serde_json::json!({ "queued": true }),
            )
            .ok();
        window
            .emit(
                "agent-event",
                serde_json::json!({
                    "sessionId": session_id,
                    "runId": run_id.clone(),
                    "event": {
                        "type": "RunEvent",
                        "event": {
                            "type": "GuidanceQueued",
                            "run_id": run_id.clone(),
                            "count": 1
                        }
                    }
                }),
            )
            .ok();
        return Ok(String::new());
    }

    let response = run_agent(
        message,
        history,
        state.clone(),
        window,
        Some(session_id.clone()),
        &agent_mode,
        attachments,
        None,
    )
    .await?;

    state
        .local_db
        .save_message(
            &session_id,
            SaveMessagePayload {
                id: None,
                role: "assistant".to_string(),
                content: response.clone(),
                created_at: None,
                metadata: serde_json::json!({ "source": "agent_chat_v2" }),
            },
        )
        .map_err(|e| e.to_string())?;

    let message_count = state
        .local_db
        .get_messages(&session_id)
        .map_err(|e| e.to_string())?
        .len();
    if message_count > 24 {
        state
            .local_db
            .summarize_session(&session_id)
            .map_err(|e| e.to_string())?;
    }

    Ok(response)
}

#[tauri::command]
pub async fn agent_subagent_chat(
    session_id: String,
    agent_name: String,
    task: String,
    display_message: Option<String>,
    mode: Option<String>,
    state: State<'_, AppState>,
    window: Window,
) -> Result<String, String> {
    let profile =
        get_agent_profile(&agent_name).ok_or_else(|| format!("没有找到子代理：{agent_name}"))?;
    let agent_mode = normalize_agent_mode(mode);
    let subagent_id = format!("subagent_{}", Uuid::new_v4());
    let visible_message = display_message
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| task.trim());
    state
        .local_db
        .save_message(
            &session_id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: visible_message.to_string(),
                created_at: None,
                metadata: serde_json::json!({
                    "source": "agent_subagent_chat",
                    "mode": agent_mode,
                    "subagent": profile.metadata.name
                }),
            },
        )
        .map_err(|e| e.to_string())?;

    if let Some(run_id) = active_run_for_session(&state, &session_id).await {
        let queued_task = format!(
            "[子代理任务]\n子代理：{}\n任务：{}\n\n当前会话已有任务运行中；请把这条子代理任务作为补充要求纳入当前执行。",
            profile.metadata.name, task
        );
        queue_run_guidance(&state, &run_id, queued_task, Vec::new()).await;
        state
            .local_db
            .append_agent_run_step(
                &run_id,
                "guidance",
                "finished",
                "收到子代理任务，已排队等待当前任务处理。",
                serde_json::json!({
                    "sessionId": session_id,
                    "subagent": profile.metadata.name,
                    "content": visible_message
                }),
                serde_json::json!({ "queued": true }),
            )
            .ok();
        window
            .emit(
                "agent-event",
                serde_json::json!({
                    "sessionId": session_id,
                    "runId": run_id.clone(),
                    "event": {
                        "type": "RunEvent",
                        "event": {
                            "type": "GuidanceQueued",
                            "run_id": run_id,
                            "count": 1
                        }
                    }
                }),
            )
            .ok();
        return Ok(String::new());
    }

    emit_subagent_event(
        &window,
        &session_id,
        AgentEvent::SubAgentStarted {
            subagent_id: subagent_id.clone(),
            name: profile.metadata.name.clone(),
            description: profile.metadata.description.clone(),
            task: task.clone(),
        },
    );

    let subagent_role =
        SubAgentRole::classify(&profile.metadata.name, &profile.metadata.description);
    let mut history = build_conversation_context(&session_id, &state, &agent_mode)?;
    let agent_instruction = Message::plain(
        Role::System,
        format!(
            "{}\n\n[子代理角色] 你以「{}」身份运行。该角色的工具权限由 Atlas 运行时强制约束：越权调用（超出角色的写入、命令或联网操作）会被直接拒绝。不要尝试超出角色的操作；只产出可被主 Agent 复核整合的结果。",
            build_subagent_instruction(&profile, &agent_mode),
            subagent_role.label_zh(),
        ),
    );
    let insertion_index = history
        .iter()
        .position(|message| !matches!(message.role, Role::System))
        .unwrap_or(history.len());
    history.insert(insertion_index, agent_instruction);
    let backend_message = format!(
        "[子代理任务]\n子代理：{}\n任务：{}\n\n请按该子代理职责处理，只给可被主 Agent 汇总的结果。",
        profile.metadata.name, task
    );

    let result = run_agent(
        backend_message,
        history,
        state.clone(),
        window.clone(),
        Some(session_id.clone()),
        &agent_mode,
        Vec::new(),
        Some(subagent_role),
    )
    .await;

    match result {
        Ok(response) => {
            state
                .local_db
                .save_message(
                    &session_id,
                    SaveMessagePayload {
                        id: None,
                        role: "assistant".to_string(),
                        content: response.clone(),
                        created_at: None,
                        metadata: serde_json::json!({
                            "source": "agent_subagent_chat",
                            "subagent": profile.metadata.name
                        }),
                    },
                )
                .map_err(|e| e.to_string())?;
            emit_subagent_event(
                &window,
                &session_id,
                AgentEvent::SubAgentFinished {
                    subagent_id,
                    name: profile.metadata.name,
                    summary: response.chars().take(220).collect(),
                },
            );
            Ok(response)
        }
        Err(error) => {
            let error_text = error.to_string();
            emit_subagent_event(
                &window,
                &session_id,
                AgentEvent::SubAgentFailed {
                    subagent_id,
                    name: profile.metadata.name,
                    error: error_text.clone(),
                },
            );
            Err(error_text)
        }
    }
}

#[tauri::command]
pub async fn summarize_session(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<crate::storage::SessionSummary, String> {
    state
        .local_db
        .summarize_session(&session_id)
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsageSnapshot {
    pub session_id: Option<String>,
    pub used_tokens: usize,
    pub limit_tokens: usize,
    pub ratio: f64,
    pub source: String,
    pub compression_state: String,
    pub summary_included: bool,
    pub message_count: usize,
    pub updated_at: i64,
}

#[tauri::command]
pub async fn get_context_usage(
    session_id: String,
    draft_message: Option<String>,
    mode: Option<String>,
    attachments: Option<Vec<AgentAttachment>>,
    state: State<'_, AppState>,
) -> Result<ContextUsageSnapshot, String> {
    let agent_mode = normalize_agent_mode(mode);
    let draft = draft_message.unwrap_or_default();
    context_usage_snapshot_for_message(
        &state.local_db,
        &session_id,
        &agent_mode,
        &draft,
        attachments.unwrap_or_default(),
    )
}

#[tauri::command]
pub async fn get_agent_skill_metadata(
    project_root: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<SkillMetadata>, String> {
    let project_root = project_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .and_then(|path| path.canonicalize().ok())
        .filter(|path| path.is_dir());
    Ok(skill_snapshot_for_project_root(&state.local_db, project_root.as_deref())?.metadata)
}

#[tauri::command]
pub async fn get_agent_subagent_metadata() -> Result<Vec<AgentProfileMetadata>, String> {
    Ok(list_agent_profile_metadata())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkillInput {
    pub name: String,
    #[serde(default)]
    pub label_zh: String,
    pub description: String,
    #[serde(default)]
    pub description_zh: String,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    pub body: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[tauri::command]
pub async fn get_agent_skill_input(
    name: String,
    state: State<'_, AppState>,
) -> Result<AgentSkillInput, String> {
    let name = sanitize_skill_name(&name)?;
    let snapshot = skill_snapshot_for_project_root(&state.local_db, None)?;
    let existing = snapshot
        .metadata
        .iter()
        .find(|skill| skill.name == name)
        .cloned()
        .ok_or_else(|| format!("没有找到 Skill：{name}"))?;
    if existing.built_in {
        return Err("内置 Skill 不能编辑。".to_string());
    }
    if existing.source_kind != "user" && existing.source_kind != "project" {
        return Err("这个 Skill 来源暂不支持编辑。".to_string());
    }
    let path = existing
        .path
        .as_deref()
        .ok_or_else(|| format!("Skill 缺少文件路径：{name}"))?;
    let content = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let skill = parse_skill_markdown(path, &content)?;
    Ok(AgentSkillInput {
        name: skill.metadata.name,
        label_zh: skill.metadata.label_zh,
        description: skill.metadata.description,
        description_zh: skill.metadata.description_zh,
        triggers: skill.metadata.triggers,
        allowed_tools: skill.metadata.allowed_tools,
        body: skill.instructions,
        overwrite: true,
    })
}

async fn build_backend_history(
    mut history: Vec<Message>,
    state: &State<'_, AppState>,
) -> Vec<Message> {
    let mut injected = Vec::new();

    if let Ok(profile) = state.local_db.get_profile() {
        injected.push(Message::plain(
            Role::System,
            format!("本地用户画像 JSON：{}", profile.profile),
        ));
    }

    if memory_injection_enabled(&state.local_db) {
        // P2-5: decay/purge before reading (see the build_context injection path).
        let _ = state.local_db.maintain_memories();
        if let Ok(memories) = state.local_db.list_memories() {
            let enabled: Vec<String> = memories
                .into_iter()
                .filter(|memory| memory.enabled)
                .map(|memory| format!("- {}（来源：{}）", memory.text, memory.source))
                .collect();
            if !enabled.is_empty() {
                let _ = state.local_db.mark_enabled_memories_used();
                injected.push(Message::plain(
                    Role::System,
                    format!("启用的本地长期记忆：\n{}", enabled.join("\n")),
                ));
            }
        }
    }

    injected.extend(history.drain(..).map(strip_historical_image_attachments));
    injected
}

/// P3-3: build a fallback-aware LLM client from the route decision's chain.
/// Builds one client per chain connection (skipping any that fail to build,
/// e.g. missing credentials). If the chain is empty or none build, falls back to
/// a single client for the already-selected config (pre-P3-3 behavior).
fn build_fallback_llm_client(
    base_config: &Config,
    decision: &ModelRouteDecision,
    db: &LocalDb,
) -> Result<Box<dyn LLMClient>, crate::config::ConfigError> {
    let mut entries: Vec<FallbackClientEntry> = Vec::new();
    for connection_id in &decision.fallback_chain {
        let mut cfg = base_config.clone();
        cfg.llm.default_connection_id = Some(connection_id.clone());
        if let Some(connection) = cfg
            .llm
            .connections
            .iter()
            .find(|connection| &connection.id == connection_id)
        {
            cfg.llm.default_provider = connection.provider_id.clone();
        }
        cfg.llm.sync_legacy_slots_from_connections();
        let (provider, model, label) = cfg
            .llm
            .active_connection()
            .map(|connection| {
                (
                    connection.provider_id.clone(),
                    connection.model.clone(),
                    format!("{}/{}", connection.provider_id, connection.model),
                )
            })
            .unwrap_or_else(|| {
                (
                    cfg.llm.default_provider.clone(),
                    connection_id.clone(),
                    connection_id.clone(),
                )
            });
        if let Ok(client) = create_llm_client(&cfg, Some(db)) {
            entries.push(FallbackClientEntry::new(
                label,
                connection_id.clone(),
                provider,
                model,
                client,
            ));
        }
    }
    if entries.is_empty() {
        let client = create_llm_client(base_config, Some(db))?;
        let (provider, model, label) = base_config
            .llm
            .active_connection()
            .map(|connection| {
                (
                    connection.provider_id.clone(),
                    connection.model.clone(),
                    format!("{}/{}", connection.provider_id, connection.model),
                )
            })
            .unwrap_or_else(|| {
                (
                    base_config.llm.default_provider.clone(),
                    "default".to_string(),
                    "default".to_string(),
                )
            });
        let connection_id = base_config
            .llm
            .default_connection_id
            .clone()
            .unwrap_or_default();
        entries.push(FallbackClientEntry::new(
            label,
            connection_id,
            provider,
            model,
            client,
        ));
    }
    Ok(Box::new(FallbackLLMClient::new(entries)))
}

fn build_agent_token_budget_snapshot(
    db: &LocalDb,
    run_id: &str,
    session_id: Option<&str>,
) -> TokenBudgetSnapshot {
    let policy = agent_token_budget_policy(db);
    if !policy.enabled {
        return TokenBudgetSnapshot::disabled();
    }

    let run_spent = db.model_usage_total_for_run(run_id).unwrap_or(0);
    let session_spent = session_id
        .map(|id| db.model_usage_total_for_session(id).unwrap_or(0))
        .unwrap_or(0);
    let day_spent = db
        .model_usage_total_since(start_of_local_day_ms())
        .unwrap_or(0);

    let mut budgets = vec![TokenBudget {
        scope: TokenBudgetScope::Run,
        soft_limit_tokens: Some(policy.run_soft_limit_tokens),
        hard_limit_tokens: Some(policy.run_hard_limit_tokens),
        spent_tokens: run_spent,
        on_hard_limit: policy.on_hard_limit,
    }];
    if session_id.is_some() {
        budgets.push(TokenBudget {
            scope: TokenBudgetScope::Session,
            soft_limit_tokens: Some(policy.session_soft_limit_tokens),
            hard_limit_tokens: Some(policy.session_hard_limit_tokens),
            spent_tokens: session_spent,
            on_hard_limit: policy.on_hard_limit,
        });
    }
    budgets.push(TokenBudget {
        scope: TokenBudgetScope::Day,
        soft_limit_tokens: Some(policy.day_soft_limit_tokens),
        hard_limit_tokens: Some(policy.day_hard_limit_tokens),
        spent_tokens: day_spent,
        on_hard_limit: policy.on_hard_limit,
    });

    TokenBudgetSnapshot::active(
        budgets,
        TokenBudgetCircuitBreaker {
            enabled: policy.circuit_breaker_enabled,
            high_total_tokens: policy.breaker_high_total_tokens,
            low_output_tokens: policy.breaker_low_output_tokens,
            consecutive_low_yield_trigger: policy.breaker_consecutive_low_yield_trigger,
            on_trigger: policy.on_hard_limit,
        },
    )
}

fn build_model_route_request(
    message: &str,
    history: &[Message],
    agent_mode: &str,
    attachments: &[AgentAttachment],
) -> ModelRouteRequest {
    let history_chars = history
        .iter()
        .map(|message| message.content.chars().count())
        .sum::<usize>();
    let image_attachment_count = attachments
        .iter()
        .filter(|attachment| is_smoke_image_attachment(attachment))
        .count();
    ModelRouteRequest::new(
        agent_mode,
        message,
        history.len(),
        history_chars,
        attachments.len(),
        image_attachment_count,
        route_request_needs_tools(message, agent_mode),
    )
}

fn route_request_needs_tools(message: &str, agent_mode: &str) -> bool {
    if agent_mode != "chat" {
        return true;
    }
    let normalized = message.to_lowercase();
    [
        "agent",
        "bug",
        "code",
        "command",
        "commit",
        "create",
        "debug",
        "execute",
        "file",
        "fix",
        "folder",
        "repo",
        "repository",
        "run",
        "search",
        "test",
        "verify",
        "write",
        "代码",
        "创建",
        "读取",
        "目录",
        "执行",
        "文件",
        "修复",
        "运行",
        "搜索",
        "提交",
        "项目",
        "验证",
        "测试",
        "命令",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn agent_model_routing_policy(db: &LocalDb) -> ModelRoutePolicy {
    let Some(value) = db
        .get_app_state(AGENT_MODEL_ROUTING_POLICY_STATE_KEY)
        .ok()
        .flatten()
    else {
        return ModelRoutePolicy::default();
    };
    if value == serde_json::Value::Bool(false) {
        return ModelRoutePolicy {
            enabled: false,
            ..ModelRoutePolicy::default()
        };
    }
    serde_json::from_value(value).unwrap_or_default()
}

fn persist_model_route_decision(
    db: &LocalDb,
    run_id: &str,
    request: &ModelRouteRequest,
    decision: &ModelRouteDecision,
) {
    let _ = db.append_agent_run_step(
        run_id,
        "model_route",
        "finished",
        &decision.summary(),
        serde_json::to_value(request).unwrap_or_else(|_| serde_json::json!({})),
        serde_json::to_value(decision).unwrap_or_else(|_| serde_json::json!({})),
    );
}

fn agent_token_budget_policy(db: &LocalDb) -> AgentTokenBudgetPolicy {
    let Some(value) = db
        .get_app_state(AGENT_TOKEN_BUDGET_POLICY_STATE_KEY)
        .ok()
        .flatten()
    else {
        return AgentTokenBudgetPolicy::default();
    };
    if value == serde_json::Value::Bool(false) {
        return AgentTokenBudgetPolicy {
            enabled: false,
            ..AgentTokenBudgetPolicy::default()
        };
    }
    serde_json::from_value(value).unwrap_or_default()
}

fn start_of_local_day_ms() -> i64 {
    let now = chrono::Local::now();
    let Some(start_naive) = now.date_naive().and_hms_opt(0, 0, 0) else {
        return 0;
    };
    start_naive
        .and_local_timezone(chrono::Local)
        .single()
        .map(|start| start.timestamp_millis())
        .unwrap_or(0)
}

fn should_mark_successful_run_finished(current_status: Option<&str>) -> bool {
    matches!(current_status, None | Some("pending" | "running"))
}

// Internal agent-run orchestration entry point: it threads request, session,
// mode, attachments and (P3-2) the subagent role through one path shared by main
// and subagent runs. Bundling these into a struct would add indirection without
// clarity, so the argument count is allowed here.
#[allow(clippy::too_many_arguments)]
async fn run_agent(
    message: String,
    history: Vec<Message>,
    state: State<'_, AppState>,
    window: Window,
    event_session_id: Option<String>,
    agent_mode: &str,
    attachments: Vec<AgentAttachment>,
    subagent_role: Option<SubAgentRole>,
) -> Result<String, String> {
    let (tx, mut rx) = mpsc::channel(32);
    let mut config = state.config.lock().await.clone();
    let smoke_model_send_base_url =
        apply_model_send_smoke_config(&mut config, &message, &attachments);
    let model_route_request =
        build_model_route_request(&message, &history, agent_mode, &attachments);
    let mut model_route_policy = agent_model_routing_policy(&state.local_db);
    if smoke_model_send_base_url.is_some() && model_route_policy.force_connection_id.is_none() {
        model_route_policy.force_connection_id =
            Some("openai-compatible:aura-smoke-model-send".to_string());
    }
    let model_route_decision = select_model_route(
        &config,
        &state.local_db,
        &model_route_request,
        &model_route_policy,
    );
    model_route_decision.apply_to_config(&mut config);
    let usage_provider = config
        .llm
        .active_connection()
        .map(|connection| connection.provider_id.clone())
        .unwrap_or_else(|| config.llm.default_provider.clone());
    let usage_model = configured_model_name(&config);
    // P3-3: build the fallback chain client from the route decision. The first
    // link is the selected connection (already applied above); the rest are
    // eligible alternates tried in order if an earlier one fails at call time.
    let llm_client = build_fallback_llm_client(&config, &model_route_decision, &state.local_db)
        .map_err(|e| e.to_string())?;
    let permission_mode = agent_permission_mode(&state.local_db);
    let tool_access_policy = tool_access_policy_for(agent_mode, &permission_mode);
    let project_root = event_session_id
        .as_deref()
        .and_then(|session_id| {
            state
                .local_db
                .session_project_root(session_id)
                .ok()
                .flatten()
        })
        .or_else(|| infer_project_root(&message, &history));
    let run_id = format!("run_{}", Uuid::new_v4());
    let tool_registry = create_runtime_tool_registry(
        state.local_db.clone(),
        state.cancel_tokens.clone(),
        event_session_id.clone(),
        Some(run_id.clone()),
        project_root.clone(),
        config.execution.clone(),
    );
    let rule_context = load_agent_rule_context(project_root.as_deref());
    // P1-6: fold a project snapshot (stack / how to build-test) into the rule prompt
    // so the agent perceives the project instead of guessing. Bounded scan,
    // dependency/build dirs excluded; only when a project root is known.
    let project_context_prompt = project_root
        .as_deref()
        .map(|root| scan_project_snapshot(root).context_prompt());
    let combined_rule_prompt = match (rule_context.prompt.clone(), project_context_prompt) {
        (Some(rule), Some(project)) => Some(format!("{rule}\n\n{project}")),
        (Some(rule), None) => Some(rule),
        (None, project) => project,
    };
    let mut skill_snapshot =
        skill_snapshot_for_project_root(&state.local_db, project_root.as_deref())?;
    if !should_auto_apply_skills(&state.local_db, &message) {
        skill_snapshot.registry = SkillRegistry::default();
    }
    state
        .local_db
        .create_agent_run(&run_id, event_session_id.as_deref(), &permission_mode)
        .map_err(|e| e.to_string())?;
    if let Some(root) = project_root.as_deref() {
        match WorkspaceLifecycleRuntime::new(state.local_db.clone()).create(
            WorkspaceLifecycleSpec {
                id: None,
                session_id: event_session_id.clone(),
                run_id: Some(run_id.clone()),
                root_path: root.to_string_lossy().to_string(),
                sandbox_backend: Some("local".to_string()),
                setup_script: None,
            },
        ) {
            Ok(workspace) => {
                let _ = state.local_db.append_agent_run_step(
                    &run_id,
                    "event",
                    "finished",
                    "Workspace lifecycle bound to agent run",
                    serde_json::json!({
                        "workspaceId": workspace.id,
                        "rootPath": workspace.root_path,
                        "sandboxStatus": workspace.sandbox_status,
                        "fallbackReason": workspace.fallback_reason,
                    }),
                    serde_json::json!({ "status": workspace.status }),
                );
            }
            Err(error) => {
                let _ = state.local_db.append_agent_run_step(
                    &run_id,
                    "event",
                    "failed",
                    "Workspace lifecycle binding failed",
                    serde_json::json!({
                        "projectRoot": root.to_string_lossy(),
                    }),
                    serde_json::json!({ "error": error.to_string() }),
                );
            }
        }
    }
    let token_budget_snapshot =
        build_agent_token_budget_snapshot(&state.local_db, &run_id, event_session_id.as_deref());
    persist_model_route_decision(
        &state.local_db,
        &run_id,
        &model_route_request,
        &model_route_decision,
    );
    let _ = record_model_route_decision_inner(&state.local_db, &model_route_decision);
    if let Some(session_id) = event_session_id.as_deref() {
        let mut active = state.active_session_runs.lock().await;
        active.insert(session_id.to_string(), run_id.clone());
    }
    let (audit_tx, mut audit_rx) = mpsc::channel::<LogAgentToolAuditPayload>(128);
    let audit_db = state.local_db.clone();
    let audit_handle = tokio::spawn(async move {
        while let Some(payload) = audit_rx.recv().await {
            // P0-4: derive an independent, structured permission decision from
            // the same event and record it in its own queryable table — not by
            // reusing the tool-audit reason field. One funnel covers every
            // gate/policy/skill decision (they all flow through this sink).
            if let Some(decision) = permission_decision_from_audit(&payload) {
                let _ = audit_db.log_permission_decision(decision);
            }
            let _ = audit_db.log_agent_tool_audit_event(payload);
        }
    });
    let audit_session_id = event_session_id.clone();
    let audit_permission_mode = permission_mode.clone();
    let audit_sink = move |event: AgentToolAuditEvent| {
        let _ = audit_tx.try_send(LogAgentToolAuditPayload {
            session_id: audit_session_id.clone(),
            run_id: event.run_id,
            iteration: event.iteration,
            tool_call_id: event.tool_call_id,
            tool_name: event.tool_name,
            permission_mode: audit_permission_mode.clone(),
            policy: event.policy,
            status: event.status.as_str().to_string(),
            reason: event.reason,
        });
    };
    let (usage_tx, mut usage_rx) = mpsc::channel::<LogModelUsagePayload>(64);
    let usage_db = state.local_db.clone();
    let usage_handle = tokio::spawn(async move {
        while let Some(payload) = usage_rx.recv().await {
            let _ = usage_db.log_model_usage_event(payload);
        }
    });
    let usage_session_id = event_session_id.clone();
    let usage_sink_provider = usage_provider.clone();
    let usage_sink_model = usage_model.clone();
    let usage_sink = move |event: crate::agent::AgentUsageEvent| {
        // M-7: when the client reported the connection that actually served the
        // turn (a fallback downgrade), bill that model; otherwise fall back to
        // the preselected route head.
        let provider = event
            .provider
            .clone()
            .unwrap_or_else(|| usage_sink_provider.clone());
        let model = event
            .model
            .clone()
            .unwrap_or_else(|| usage_sink_model.clone());
        let _ = usage_tx.try_send(LogModelUsagePayload {
            session_id: usage_session_id.clone(),
            run_id: event.run_id,
            iteration: event.iteration,
            provider,
            model,
            input_tokens: event.usage.input_tokens,
            output_tokens: event.usage.output_tokens,
            total_tokens: event.usage.total_tokens,
            source: event.source,
        });
    };
    let active_task_db = state.local_db.clone();
    let active_task_session = event_session_id.clone();
    let active_task_context_db = state.local_db.clone();
    let active_task_context_session = event_session_id.clone();
    let append_delivery_report = should_append_delivery_report_to_chat(&state.local_db);
    let final_audit_db = state.local_db.clone();
    let final_audit_session = event_session_id.clone();
    let final_audit_run_id = run_id.clone();
    // P2-1: captures for the auto-verify-after-command hook (main-loop verify).
    let verify_db = state.local_db.clone();
    let verify_session = event_session_id.clone();
    let verify_root = project_root.clone();
    let mut agent = Agent::new(llm_client, tool_registry)
        .with_tools_enabled(true)
        .with_tool_access_policy(tool_access_policy)
        .with_subagent_role(subagent_role)
        .with_tool_audit_sink(audit_sink)
        .with_usage_sink(usage_sink)
        .with_run_id(run_id.clone())
        .with_token_budget_snapshot(token_budget_snapshot)
        .with_guidance_queues(state.run_guidance.clone())
        .with_pause_registry(state.pause_registry.clone())
        .with_skill_registry(skill_snapshot.registry)
        .with_rule_prompt(combined_rule_prompt)
        .with_active_task_provider(move || {
            let session_id = active_task_session.as_deref()?;
            match active_task_db.get_active_plan_task(session_id) {
                Ok(Some(task)) => Some(task.id),
                _ => None,
            }
        })
        .with_active_task_context_provider(move || {
            let session_id = active_task_context_session.as_deref()?;
            active_plan_task_context_note(&active_task_context_db, session_id)
                .ok()
                .flatten()
        })
        .with_final_audit_provider(move |goal: &str| {
            if !append_delivery_report {
                return None;
            }
            let session_id = final_audit_session.as_deref()?;
            match crate::agent::final_audit::compute_delivery_report(
                &final_audit_db,
                session_id,
                Some(&final_audit_run_id),
                goal,
            ) {
                Ok(report) => serde_json::to_value(&report).ok(),
                Err(_) => {
                    // P2-13 rollback path: if DeliveryReport assembly breaks,
                    // keep the older final audit surface rather than dropping
                    // the hard-block footer/event entirely.
                    crate::agent::final_audit::compute_final_audit(
                        &final_audit_db,
                        session_id,
                        goal,
                    )
                    .ok()
                    .and_then(|audit| serde_json::to_value(&audit).ok())
                }
            }
        })
        .with_post_command_verify_hook(move |command: String| {
            // Clone per-invocation: the hook is Fn (callable many times per run).
            let db = verify_db.clone();
            let session = verify_session.clone();
            let root = verify_root.clone();
            async move {
                let session = session?;
                let task = db.get_active_plan_task(&session).ok().flatten()?;
                let root_path = root.as_deref().map(std::path::Path::new);
                // Matcher gate: opt-in via `auto_after_command` in verify config.
                let cfg = crate::agent::verification::load_verify_config(
                    root_path,
                    dirs::home_dir().as_deref(),
                    Some(&task.verify),
                );
                let matcher = cfg.auto_after_command.as_deref()?;
                let re = regex::Regex::new(matcher).ok()?;
                if !re.is_match(&command) {
                    return None;
                }
                crate::tools::run_verify::verify_after_command(&db, &task, root_path).await
            }
        });
    let cancel_token = CancellationToken::new();
    let cancel_key = cancel_key_for(event_session_id.as_deref());
    let proof_message = message.clone();
    let proof_attachments = attachments.clone();
    {
        let mut active = state.cancel_tokens.lock().await;
        active.insert(cancel_key.clone(), cancel_token.clone());
    }
    {
        // P1-2: 为本 run 注册暂停句柄(按 run_id 索引,与 core 循环读取一致)。
        let mut registry = state.pause_registry.lock().await;
        registry.insert(
            run_id.clone(),
            std::sync::Arc::new(crate::agent::RunPauseHandle::new()),
        );
    }

    let cancel_event_tx = tx.clone();
    let cancel_event_run_id = run_id.clone();
    let route_event_tx = tx.clone();
    let route_summary = model_route_decision.summary();
    let chat_handle = tokio::spawn(async move {
        tokio::select! {
            result = agent.chat_with_history_with_attachments(message, history, attachments, tx) => result,
            _ = cancel_token.cancelled() => {
                let _ = cancel_event_tx.send(AgentEvent::RunEvent {
                    event: AgentRunEvent::Cancelled {
                        run_id: cancel_event_run_id,
                    },
                }).await;
                Err(crate::agent::AgentError::Cancelled)
            },
        }
    });

    let event_db = state.local_db.clone();
    let event_run_id = run_id.clone();
    let emit_session_id = event_session_id.clone();
    let event_handle = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            persist_agent_event(&event_db, &event_run_id, emit_session_id.as_deref(), &event);
            window
                .emit(
                    "agent-event",
                    serde_json::json!({
                        "sessionId": emit_session_id.clone(),
                        "runId": event_run_id.clone(),
                        "event": event
                    }),
                )
                .ok();
        }
    });
    let _ = route_event_tx.try_send(AgentEvent::Thinking {
        content: route_summary,
    });

    let agent_result = match chat_handle.await {
        Ok(result) => result,
        Err(error) => {
            drop(route_event_tx);
            let _ = event_handle.await;
            return Err(error.to_string());
        }
    };
    drop(route_event_tx);
    let _ = event_handle.await;

    let mut active = state.cancel_tokens.lock().await;
    active.remove(&cancel_key);
    drop(active);
    // P1-2: run 结束(完成 / 失败 / 取消)后丢弃暂停句柄,避免 registry 泄漏。
    state.pause_registry.lock().await.remove(&run_id);
    if let Some(session_id) = event_session_id.as_deref() {
        let mut active_runs = state.active_session_runs.lock().await;
        if active_runs
            .get(session_id)
            .is_some_and(|active| active == &run_id)
        {
            active_runs.remove(session_id);
        }
    }
    let leftover_guidance = state
        .run_guidance
        .lock()
        .await
        .remove(&run_id)
        .unwrap_or_default();
    if !leftover_guidance.is_empty() {
        let _ = state.local_db.append_agent_run_step(
            &run_id,
            "guidance",
            "finished",
            &format!(
                "任务结束时还有 {} 条补充消息未进入模型调用；这些消息已保存在会话历史中。",
                leftover_guidance.len()
            ),
            serde_json::json!({ "count": leftover_guidance.len() }),
            serde_json::json!({ "merged": false }),
        );
    }
    match &agent_result {
        Ok(_) => {
            let current_status = state
                .local_db
                .get_agent_run(&run_id)
                .ok()
                .flatten()
                .map(|run| run.status);
            if should_mark_successful_run_finished(current_status.as_deref()) {
                let _ = state
                    .local_db
                    .update_agent_run_status(&run_id, "finished", None);
            }
        }
        Err(AgentError::Cancelled) => {
            let _ = state.local_db.update_agent_run_status(
                &run_id,
                "cancelled",
                Some("用户取消任务。"),
            );
        }
        Err(error) => {
            let error_text = error.to_string();
            let _ = state
                .local_db
                .update_agent_run_status(&run_id, "failed", Some(&error_text));
        }
    }
    let _ = audit_handle.await;
    let _ = usage_handle.await;

    if let Ok(response) = &agent_result {
        write_island_model_send_smoke_proof(
            event_session_id.as_deref(),
            &run_id,
            agent_mode,
            &usage_provider,
            &usage_model,
            smoke_model_send_base_url.as_deref(),
            &proof_message,
            &proof_attachments,
            response,
        );
    }

    agent_result.map_err(|e| e.to_string())
}

fn apply_model_send_smoke_config(
    config: &mut Config,
    message: &str,
    attachments: &[AgentAttachment],
) -> Option<String> {
    let base_url = atlas_model_send_smoke_base_url(message).or_else(|| {
        if message_has_island_context(message) || attachments.iter().any(has_island_package_id) {
            island_model_send_smoke_base_url()
        } else {
            None
        }
    })?;

    let connection = ModelConnectionConfig {
        id: "openai-compatible:aura-smoke-model-send".to_string(),
        name: "Aura smoke model-send fixture".to_string(),
        provider_id: "openai".to_string(),
        route_id: "aura-smoke-model-send".to_string(),
        protocol: "openai-compatible".to_string(),
        api_key: "aura-smoke-key".to_string(),
        model: "aura-smoke-model-send".to_string(),
        base_url: Some(base_url.clone()),
        enabled: true,
        auth_header: None,
    };
    config.llm.upsert_connection(connection);
    config.llm.default_provider = "openai".to_string();
    config.llm.default_connection_id = Some("openai-compatible:aura-smoke-model-send".to_string());
    Some(base_url)
}

fn atlas_model_send_smoke_base_url(message: &str) -> Option<String> {
    if std::env::var("AURA_SMOKE_ENABLE_ATLAS_MODEL_SEND_PROOF")
        .ok()
        .as_deref()
        != Some("1")
    {
        return None;
    }
    if std::env::var("AURA_SMOKE_RUN_ID")
        .ok()
        .map(|value| value.trim().is_empty())
        .unwrap_or(true)
    {
        return None;
    }
    if !std::env::var("AURA_HOME")
        .ok()
        .map(|value| value.to_ascii_lowercase().contains("tauri-smoke"))
        .unwrap_or(false)
    {
        return None;
    }
    if message.contains("[Atlas Rich Run smoke]") || message.contains("[Atlas Quick Reply smoke]") {
        return rich_atlas_model_send_smoke_base_url().or_else(local_smoke_model_send_base_url);
    }
    local_smoke_model_send_base_url()
}

fn island_model_send_smoke_base_url() -> Option<String> {
    if std::env::var("AURA_SMOKE_ENABLE_ISLAND_MODEL_SEND_PROOF")
        .ok()
        .as_deref()
        != Some("1")
    {
        return None;
    }
    if std::env::var("AURA_SMOKE_RUN_ID")
        .ok()
        .map(|value| value.trim().is_empty())
        .unwrap_or(true)
    {
        return None;
    }
    local_smoke_model_send_base_url()
}

fn rich_atlas_model_send_smoke_base_url() -> Option<String> {
    std::env::var("AURA_SMOKE_ATLAS_RICH_MODEL_SEND_BASE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| {
            value.starts_with("http://127.0.0.1:") || value.starts_with("http://localhost:")
        })
}

fn local_smoke_model_send_base_url() -> Option<String> {
    std::env::var("AURA_SMOKE_MODEL_SEND_BASE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| {
            value.starts_with("http://127.0.0.1:") || value.starts_with("http://localhost:")
        })
}

fn message_has_island_context(message: &str) -> bool {
    message.contains("[Aura 浮层上下文]")
}

fn has_island_package_id(attachment: &AgentAttachment) -> bool {
    attachment
        .island_package_id
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn is_smoke_image_attachment(attachment: &AgentAttachment) -> bool {
    attachment.kind.eq_ignore_ascii_case("image")
        || attachment.mime.to_lowercase().starts_with("image/")
        || attachment
            .data_url
            .as_deref()
            .map(|value| value.starts_with("data:image/"))
            .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn write_island_model_send_smoke_proof(
    session_id: Option<&str>,
    run_id: &str,
    agent_mode: &str,
    provider: &str,
    model: &str,
    base_url_override: Option<&str>,
    message: &str,
    attachments: &[AgentAttachment],
    response: &str,
) {
    let Some(base_url_override) = base_url_override else {
        return;
    };
    let smoke_run_id = match std::env::var("AURA_SMOKE_RUN_ID") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return,
    };
    if !message_has_island_context(message) && !attachments.iter().any(has_island_package_id) {
        return;
    }

    let island_package_ids = attachments
        .iter()
        .filter_map(|attachment| attachment.island_package_id.as_deref())
        .filter(|value| !value.trim().is_empty())
        .map(|value| truncate_smoke_text(value, 120))
        .collect::<Vec<_>>();
    let image_attachment_count = attachments
        .iter()
        .filter(|attachment| is_smoke_image_attachment(attachment))
        .count();
    let source = if message.contains("来源：截图") {
        "screenshot"
    } else if message.contains("来源：OCR") {
        "ocr"
    } else if message.contains("来源：当前窗口") {
        "window_context"
    } else if message.contains("来源：剪贴板") {
        "clipboard"
    } else if message.contains("来源：系统状态") {
        "system"
    } else if message.contains("来源：文件") {
        "file"
    } else {
        "unknown"
    };
    let captured_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let proof = serde_json::json!({
        "kind": "island_context_model_send_smoke_proof",
        "ok": true,
        "smokeRunId": smoke_run_id,
        "sessionId": session_id.unwrap_or(""),
        "runId": run_id,
        "mode": agent_mode,
        "provider": provider,
        "model": model,
        "baseUrlOverride": truncate_smoke_text(base_url_override, 160),
        "source": source,
        "messageContainsIslandContext": message_has_island_context(message),
        "messageContainsSourceLabel": source != "unknown",
        "messageSnippet": truncate_smoke_text(message, 260),
        "attachmentCount": attachments.len(),
        "imageAttachmentCount": image_attachment_count,
        "attachmentsHaveImageDataUrl": attachments
            .iter()
            .any(|attachment| attachment.data_url.as_deref().is_some_and(|value| value.starts_with("data:image/"))),
        "islandPackageIds": island_package_ids,
        "islandPackageIdCount": attachments.iter().filter(|attachment| has_island_package_id(attachment)).count(),
        "responseLength": response.chars().count(),
        "modelSendSucceeded": true,
        "capturedAt": captured_at_ms
    });
    let path = std::env::temp_dir().join(format!(
        "aura-island-model-send-smoke-{}-{}.json",
        proof
            .get("smokeRunId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown"),
        Uuid::new_v4()
    ));
    if let Ok(bytes) = serde_json::to_vec(&proof) {
        let _ = std::fs::write(path, bytes);
    }
}

fn truncate_smoke_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn configured_model_name(config: &crate::config::Config) -> String {
    config
        .llm
        .active_connection()
        .map(|connection| connection.model.clone())
        .unwrap_or_else(|| "unknown".to_string())
}

#[tauri::command]
pub async fn get_model_usage_summary(
    session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ModelUsageSummary, String> {
    state
        .local_db
        .model_usage_summary(session_id.as_deref())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_agent_runs(
    session_id: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<AgentRunRecord>, String> {
    state
        .local_db
        .recent_agent_runs(session_id.as_deref(), limit.unwrap_or(20))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_agent_run_steps(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<AgentRunStepRecord>, String> {
    state
        .local_db
        .get_agent_run_steps(&run_id)
        .map_err(|error| error.to_string())
}

/// P1-3: replay a run's full event timeline (step + tool + usage + verify) in
/// chronological order. Paginated via `limit`/`offset`; the response `total` lets
/// the UI page through the entire run, not just the latest N events.
#[tauri::command]
pub async fn get_agent_run_timeline(
    run_id: String,
    limit: Option<i64>,
    offset: Option<i64>,
    state: State<'_, AppState>,
) -> Result<RunTimeline, String> {
    state
        .local_db
        .get_run_timeline(&run_id, limit.unwrap_or(200), offset.unwrap_or(0))
        .map_err(|error| error.to_string())
}

/// OS-2: browser actions are persisted as first-class agent observations, scoped
/// by run/session and queryable without reading the legacy app_state audit blob.
#[tauri::command]
pub async fn get_browser_agent_steps(
    run_id: Option<String>,
    session_id: Option<String>,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<BrowserAgentStepRecord>, String> {
    state
        .local_db
        .list_browser_agent_steps(
            run_id.as_deref(),
            session_id.as_deref(),
            limit.unwrap_or(100),
        )
        .map_err(|error| error.to_string())
}

/// OS-3: create a durable agent graph run. This only writes graph structure; real
/// execution is driven by runtime executors, not by UI placeholders.
#[tauri::command]
pub async fn create_agent_graph_run(
    spec: CreateAgentGraphSpec,
    state: State<'_, AppState>,
) -> Result<AgentGraphSnapshot, String> {
    DurableAgentGraphRuntime::new(state.local_db.clone())
        .create_run(spec)
        .map_err(|error| error.to_string())
}

/// OS-3: fetch persisted graph state including nodes, edges, and checkpoints.
#[tauri::command]
pub async fn get_agent_graph_snapshot(
    graph_run_id: String,
    state: State<'_, AppState>,
) -> Result<AgentGraphSnapshot, String> {
    state
        .local_db
        .get_agent_graph_snapshot(&graph_run_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn pause_agent_graph_run(
    graph_run_id: String,
    reason: String,
    state: State<'_, AppState>,
) -> Result<AgentGraphSnapshot, String> {
    DurableAgentGraphRuntime::new(state.local_db.clone())
        .pause_run(&graph_run_id, &reason)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resume_agent_graph_run(
    graph_run_id: String,
    reason: String,
    state: State<'_, AppState>,
) -> Result<AgentGraphSnapshot, String> {
    DurableAgentGraphRuntime::new(state.local_db.clone())
        .resume_run(&graph_run_id, &reason)
        .map_err(|error| error.to_string())
}

/// OS-4: create a durable team run with explicit participants/roles. The run is
/// only an orchestration record; subagent outputs still require main-agent review.
#[tauri::command]
pub async fn create_team_run(
    spec: CreateTeamRunSpec,
    state: State<'_, AppState>,
) -> Result<TeamRunSnapshot, String> {
    DurableTeamRuntime::new(state.local_db.clone())
        .create_run(spec)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_team_run_snapshot(
    team_run_id: String,
    state: State<'_, AppState>,
) -> Result<TeamRunSnapshot, String> {
    state
        .local_db
        .get_team_run_snapshot(&team_run_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn append_team_message(
    team_run_id: String,
    participant_id: Option<String>,
    message_type: String,
    content: String,
    metadata: Option<serde_json::Value>,
    state: State<'_, AppState>,
) -> Result<TeamMessageRecord, String> {
    DurableTeamRuntime::new(state.local_db.clone())
        .append_participant_message(
            &team_run_id,
            participant_id,
            &message_type,
            &content,
            metadata.unwrap_or_else(|| serde_json::json!({})),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn create_handoff_request(
    team_run_id: String,
    from_participant_id: Option<String>,
    to_participant_id: String,
    reason: String,
    contract: HandoffContract,
    state: State<'_, AppState>,
) -> Result<HandoffRequestRecord, String> {
    DurableTeamRuntime::new(state.local_db.clone())
        .request_handoff(
            &team_run_id,
            from_participant_id,
            to_participant_id,
            reason,
            contract,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resolve_handoff_request(
    handoff_id: String,
    status: String,
    result: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<HandoffRequestRecord, String> {
    state
        .local_db
        .resolve_handoff_request(&handoff_id, &status, result)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn apply_team_termination(
    team_run_id: String,
    state: State<'_, AppState>,
) -> Result<crate::agent::TeamTerminationVerdict, String> {
    DurableTeamRuntime::new(state.local_db.clone())
        .apply_termination(&team_run_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn schedule_team_execution_plan(
    team_run_id: String,
    options: TeamExecutionOptions,
    state: State<'_, AppState>,
) -> Result<TeamExecutionPlan, String> {
    DurableTeamRuntime::new(state.local_db.clone())
        .schedule_execution_plan(&team_run_id, options)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn pause_team_execution(
    team_run_id: String,
    reason: String,
    state: State<'_, AppState>,
) -> Result<TeamExecutionPlan, String> {
    DurableTeamRuntime::new(state.local_db.clone())
        .pause_execution(&team_run_id, &reason)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resume_team_execution(
    team_run_id: String,
    reason: String,
    state: State<'_, AppState>,
) -> Result<TeamExecutionPlan, String> {
    DurableTeamRuntime::new(state.local_db.clone())
        .resume_execution(&team_run_id, &reason)
        .map_err(|error| error.to_string())
}

/// OS-5: add source/trust/confidence tracked knowledge for retrieval. This is
/// separate from the older preference memory table and can be cited by source.
#[tauri::command]
pub async fn add_knowledge_item(
    payload: AddKnowledgeItemPayload,
    state: State<'_, AppState>,
) -> Result<KnowledgeItemRecord, String> {
    state
        .local_db
        .add_knowledge_item(payload)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn search_knowledge(
    request: KnowledgeRecallRequest,
    state: State<'_, AppState>,
) -> Result<RetrievalContext, String> {
    recall_knowledge(&state.local_db, request)
}

#[tauri::command]
pub async fn delete_knowledge_item(
    id: String,
    state: State<'_, AppState>,
) -> Result<KnowledgeItemRecord, String> {
    state
        .local_db
        .delete_knowledge_item(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn ingest_connector_knowledge_items(
    request: KnowledgeConnectorIngestRequest,
    state: State<'_, AppState>,
) -> Result<KnowledgeConnectorIngestReport, String> {
    ingest_connector_knowledge(&state.local_db, request)
}

#[tauri::command]
pub async fn write_agent_memory_event(
    event: MemoryWriteEvent,
    state: State<'_, AppState>,
) -> Result<KnowledgeItemRecord, String> {
    write_memory_from_event(&state.local_db, event)
}

/// OS-6: create/query workspace lifecycle records so runs are bound to a real
/// workspace root and sandbox fallback is explicit rather than silently assumed.
#[tauri::command]
pub async fn create_workspace_lifecycle(
    spec: WorkspaceLifecycleSpec,
    state: State<'_, AppState>,
) -> Result<WorkspaceLifecycleRecord, String> {
    WorkspaceLifecycleRuntime::new(state.local_db.clone())
        .create(spec)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_workspace_lifecycle_snapshot(
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<WorkspaceLifecycleSnapshot, String> {
    state
        .local_db
        .get_workspace_lifecycle_snapshot(&workspace_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn validate_workspace_cwd(
    workspace_id: String,
    cwd: String,
    state: State<'_, AppState>,
) -> Result<WorkspaceCwdVerdict, String> {
    let snapshot = state
        .local_db
        .get_workspace_lifecycle_snapshot(&workspace_id)
        .map_err(|error| error.to_string())?;
    validate_workspace_cwd_for_record(&snapshot.workspace, &cwd).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn run_workspace_setup_script(
    workspace_id: String,
    options: WorkspaceSetupRunOptions,
    state: State<'_, AppState>,
) -> Result<WorkspaceLifecycleSnapshot, String> {
    WorkspaceLifecycleRuntime::new(state.local_db.clone())
        .run_setup_script(&workspace_id, options)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn validate_workspace_command_binding(
    workspace_id: String,
    cwd: String,
    state: State<'_, AppState>,
) -> Result<WorkspaceCwdVerdict, String> {
    WorkspaceLifecycleRuntime::new(state.local_db.clone())
        .validate_command_binding(&workspace_id, &cwd)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn install_workspace_git_hook(
    workspace_id: String,
    spec: WorkspaceGitHookSpec,
    state: State<'_, AppState>,
) -> Result<WorkspaceGitHookInstallReport, String> {
    WorkspaceLifecycleRuntime::new(state.local_db.clone())
        .install_git_hook(&workspace_id, spec)
        .map_err(|error| error.to_string())
}

/// OS-7: export a run trajectory from the real persisted timeline. Replay is
/// read-only and never calls the model or mutating tools.
#[tauri::command]
pub async fn export_agent_run_trajectory(
    options: TrajectoryExportOptions,
    state: State<'_, AppState>,
) -> Result<TrajectoryExport, String> {
    export_run_trajectory(&state.local_db, options)
}

#[tauri::command]
pub async fn replay_agent_run_trajectory(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<ReplayReport, String> {
    let export = export_run_trajectory(
        &state.local_db,
        TrajectoryExportOptions {
            run_id,
            include_payloads: true,
            redact_secrets: true,
            limit: None,
        },
    )?;
    Ok(replay_trajectory_readonly(&export))
}

#[tauri::command]
pub async fn evaluate_agent_run_trajectory(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<TrajectoryEvalReport, String> {
    let export = export_run_trajectory(
        &state.local_db,
        TrajectoryExportOptions {
            run_id,
            include_payloads: true,
            redact_secrets: true,
            limit: None,
        },
    )?;
    Ok(evaluate_trajectory_completion(&export))
}

/// OS-8: map an external Agent Protocol / ACP / A2A task into an Aura run
/// without bypassing Aura permission, checkpoint, and final-audit boundaries.
#[tauri::command]
pub async fn create_external_agent_task(
    task: ExternalTask,
    state: State<'_, AppState>,
) -> Result<ProtocolRunMapping, String> {
    create_external_task_mapping(&state.local_db, task)
}

#[tauri::command]
pub async fn get_external_agent_task_mapping(
    protocol: String,
    external_task_id: String,
    state: State<'_, AppState>,
) -> Result<ProtocolRunMapping, String> {
    get_external_task_mapping(&state.local_db, &protocol, &external_task_id)
}

#[tauri::command]
pub async fn get_protocol_compatibility_matrix() -> Result<Vec<ProtocolCompatibilityEntry>, String>
{
    Ok(protocol_compatibility_matrix())
}

#[tauri::command]
pub async fn update_external_agent_task_lifecycle(
    update: ProtocolLifecycleUpdate,
    state: State<'_, AppState>,
) -> Result<ProtocolRunMapping, String> {
    update_external_task_lifecycle(&state.local_db, update)
}

#[tauri::command]
pub async fn cancel_external_agent_task(
    protocol: String,
    external_task_id: String,
    reason: String,
    state: State<'_, AppState>,
) -> Result<ProtocolRunMapping, String> {
    cancel_external_task(&state.local_db, &protocol, &external_task_id, &reason)
}

#[tauri::command]
pub async fn append_external_agent_task_stream_event(
    protocol: String,
    external_task_id: String,
    event_type: String,
    payload: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<Vec<ProtocolStreamEvent>, String> {
    append_external_task_stream_event(
        &state.local_db,
        &protocol,
        &external_task_id,
        &event_type,
        payload,
    )
}

#[tauri::command]
pub async fn list_external_agent_task_stream_events(
    protocol: String,
    external_task_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ProtocolStreamEvent>, String> {
    list_external_task_stream_events(&state.local_db, &protocol, &external_task_id)
}

/// OS-9: evaluate plugin/skill quality before enablement. This is backend
/// governance; marketplace UI remains deferred.
#[tauri::command]
pub async fn evaluate_plugin_quality_gate(
    request: PluginQualityGateRequest,
    state: State<'_, AppState>,
) -> Result<PluginQualityGate, String> {
    evaluate_installed_plugin_quality_gate(&state.local_db, request)
}

#[tauri::command]
pub async fn get_plugin_eval_registry_entry(
    plugin_id: String,
    state: State<'_, AppState>,
) -> Result<PluginEvalRegistryEntry, String> {
    let package = state
        .local_db
        .list_plugin_packages()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|package| package.id == plugin_id)
        .ok_or_else(|| format!("plugin package not found: {plugin_id}"))?;
    Ok(plugin_eval_registry_entry(&package))
}

#[tauri::command]
pub async fn get_skill_version_registry(
    plugin_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SkillVersionRecord>, String> {
    let package = state
        .local_db
        .list_plugin_packages()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|package| package.id == plugin_id)
        .ok_or_else(|| format!("plugin package not found: {plugin_id}"))?;
    Ok(skill_version_registry(&package))
}

#[tauri::command]
pub async fn get_team_preset_permission_report(
    plugin_id: String,
    state: State<'_, AppState>,
) -> Result<TeamPresetPermissionReport, String> {
    let package = state
        .local_db
        .list_plugin_packages()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|package| package.id == plugin_id)
        .ok_or_else(|| format!("plugin package not found: {plugin_id}"))?;
    Ok(team_preset_permission_report(&package))
}

/// OS-10: inspect code intelligence using workspace-bound LSP-shaped payloads.
/// If no LSP session is available, the response says so instead of faking
/// diagnostics from text search.
#[tauri::command]
pub async fn inspect_code_intelligence_report(
    request: CodeIntelligenceRequest,
) -> Result<CodeIntelligenceReport, String> {
    inspect_code_intelligence(request)
}

#[tauri::command]
pub async fn prepare_lsp_session_plan(request: LspSessionSpec) -> Result<LspSessionPlan, String> {
    prepare_lsp_session(request)
}

/// OS-11: provider economics primitives. Cost estimates are separated from
/// route selection until the quality feedback loop is mature enough to enforce.
#[tauri::command]
pub async fn estimate_model_cost(
    provider: String,
    model: String,
    input_tokens: i64,
    output_tokens: i64,
) -> Result<ModelCostEstimate, String> {
    Ok(estimate_cost_from_parts(
        &provider,
        &model,
        input_tokens,
        output_tokens,
        input_tokens + output_tokens,
    ))
}

#[tauri::command]
pub async fn estimate_model_text_cost(
    provider: String,
    model: String,
    input_text: String,
    output_text: Option<String>,
) -> Result<ModelTextCostEstimate, String> {
    Ok(estimate_model_text_cost_inner(
        &provider,
        &model,
        &input_text,
        output_text.as_deref().unwrap_or(""),
    ))
}

#[tauri::command]
pub async fn record_model_quality_event(
    request: RecordModelQualityEventRequest,
    state: State<'_, AppState>,
) -> Result<ModelQualityEvent, String> {
    record_model_quality_event_inner(&state.local_db, request)
}

#[tauri::command]
pub async fn get_model_quality_events(
    state: State<'_, AppState>,
) -> Result<Vec<ModelQualityEvent>, String> {
    list_model_quality_events(&state.local_db)
}

#[tauri::command]
pub async fn get_model_route_decisions(
    state: State<'_, AppState>,
) -> Result<Vec<ModelRouteDecisionAudit>, String> {
    list_model_route_decisions(&state.local_db)
}

#[tauri::command]
pub async fn explain_route_economics(
    provider: String,
    model: String,
    input_tokens: i64,
    output_tokens: i64,
    state: State<'_, AppState>,
) -> Result<crate::agent::RouteEconomicsDecision, String> {
    let estimate = estimate_cost_from_parts(
        &provider,
        &model,
        input_tokens,
        output_tokens,
        input_tokens + output_tokens,
    );
    let events = list_model_quality_events(&state.local_db)?;
    Ok(route_economics_decision(estimate, &events))
}

/// OS-12: expose graph node execution trace and a minimal durable queue without
/// building the visual editor yet.
#[tauri::command]
pub async fn get_agent_graph_node_traces(
    graph_run_id: String,
    state: State<'_, AppState>,
) -> Result<WorkflowTraceReport, String> {
    graph_node_traces(&state.local_db, &graph_run_id)
}

#[tauri::command]
pub async fn enqueue_agent_graph_run(
    graph_run_id: String,
    priority: Option<i64>,
    state: State<'_, AppState>,
) -> Result<QueuedGraphRun, String> {
    enqueue_graph_run(&state.local_db, &graph_run_id, priority)
}

#[tauri::command]
pub async fn abort_queued_agent_graph_run(
    queue_id: String,
    reason: String,
    state: State<'_, AppState>,
) -> Result<QueuedGraphRun, String> {
    abort_queued_graph_run(&state.local_db, &queue_id, &reason)
}

#[tauri::command]
pub async fn list_agent_graph_queue(
    state: State<'_, AppState>,
) -> Result<Vec<QueuedGraphRun>, String> {
    list_graph_queue(&state.local_db)
}

#[tauri::command]
pub async fn set_agent_graph_queue_paused(
    paused: bool,
    reason: String,
    state: State<'_, AppState>,
) -> Result<WorkflowQueueControl, String> {
    set_graph_queue_paused(&state.local_db, paused, &reason)
}

#[tauri::command]
pub async fn get_agent_graph_queue_control(
    state: State<'_, AppState>,
) -> Result<WorkflowQueueControl, String> {
    get_graph_queue_control(&state.local_db)
}

/// P4-1: run-scoped file diff built from checkpoint before/after snapshots.
/// This does not read current files from disk, so old run history is not
/// polluted by later user edits.
#[tauri::command]
pub async fn get_agent_run_diff(
    run_id: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<RunDiff, String> {
    build_run_diff(&state.local_db, &run_id, limit.unwrap_or(200))
        .map_err(|error| error.to_string())
}

/// P4-2: run-scoped terminal feed derived only from persisted command or
/// verification payloads. Tool names without command bodies are intentionally
/// omitted so the UI cannot display fabricated terminal rows.
#[tauri::command]
pub async fn get_agent_run_terminal(
    run_id: String,
    limit: Option<i64>,
    offset: Option<i64>,
    state: State<'_, AppState>,
) -> Result<RunTerminalFeed, String> {
    let timeline = complete_run_timeline(&state.local_db, &run_id)?;
    Ok(run_terminal_feed(
        &timeline,
        limit.unwrap_or(200),
        offset.unwrap_or(0),
    ))
}

/// P4-3: normalized audit feed for tool audit, permission decisions,
/// verification evidence, plan changes, usage, and any persisted final audit
/// payloads. The detail field keeps the original source record intact.
#[tauri::command]
pub async fn get_agent_run_audit(
    run_id: String,
    limit: Option<i64>,
    offset: Option<i64>,
    state: State<'_, AppState>,
) -> Result<RunAuditFeed, String> {
    let timeline = complete_run_timeline(&state.local_db, &run_id)?;
    Ok(run_audit_feed(
        &timeline,
        limit.unwrap_or(200),
        offset.unwrap_or(0),
    ))
}

/// P4-4/P4-6: compact progress state for reporting without UI-specific colors.
/// `semantic.tone` is a truthful status category, not a visual success claim.
#[tauri::command]
pub async fn get_agent_run_progress(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<RunProgressSummary, String> {
    let timeline = complete_run_timeline(&state.local_db, &run_id)?;
    Ok(run_progress_summary(&timeline))
}

/// P4-4: shared status semantics for UI and report code. This is deliberately a
/// data contract; visual colors remain a later UI task.
#[tauri::command]
pub async fn get_agent_status_semantic(
    domain: String,
    status: String,
) -> Result<StatusSemantic, String> {
    Ok(status_semantic(&domain, &status))
}

/// E-1..E-5: built-in evaluation suite manifests for benchmark, security,
/// false-completion, rollback, and provider compatibility tasks.
#[tauri::command]
pub async fn get_agent_eval_suites() -> Result<Vec<EvalSuite>, String> {
    builtin_eval_suites()
}

/// E-1..E-5: score an evaluation suite against real harness outcomes. A case
/// only passes when it both passed and was verified, and false completion always
/// counts against the exit gate.
#[tauri::command]
pub async fn score_agent_eval_suite(
    suite_id: String,
    outcomes: Vec<EvalCaseOutcome>,
) -> Result<EvalSuiteReport, String> {
    let suite = builtin_eval_suite(&suite_id)?;
    Ok(score_eval_suite(&suite, &outcomes))
}

/// E-1..E-5 / M-10: execute verifier commands for one built-in evaluation
/// suite and return a machine-readable run report. This runs objective
/// verifiers only; the caller controls when and how real model tasks are run.
#[tauri::command]
pub async fn run_agent_eval_suite_verifiers(
    suite_id: String,
    case_ids: Option<Vec<String>>,
    cwd: Option<String>,
    claimed_complete: Option<bool>,
    state: State<'_, AppState>,
) -> Result<EvalRunReport, String> {
    let report = run_eval_suite_verifiers(EvalRunOptions {
        suite_id,
        case_ids: case_ids.unwrap_or_default(),
        cwd,
        claimed_complete: claimed_complete.unwrap_or(false),
    })?;
    persist_eval_report(&state.local_db, &report, None, None)?;
    Ok(report)
}

fn complete_run_timeline(db: &LocalDb, run_id: &str) -> Result<RunTimeline, String> {
    let mut timeline = db
        .get_run_timeline(run_id, 1000, 0)
        .map_err(|error| error.to_string())?;
    let total = timeline.total;
    let mut entries = std::mem::take(&mut timeline.entries);
    let mut offset = entries.len() as i64;
    while offset < total {
        let mut page = db
            .get_run_timeline(run_id, 1000, offset)
            .map_err(|error| error.to_string())?;
        if page.entries.is_empty() {
            break;
        }
        entries.append(&mut page.entries);
        offset = entries.len() as i64;
    }
    timeline.offset = 0;
    timeline.limit = total.max(entries.len() as i64);
    timeline.entries = entries;
    Ok(timeline)
}

#[tauri::command]
pub async fn get_agent_tool_audit_events(
    session_id: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<AgentToolAuditRecord>, String> {
    state
        .local_db
        .recent_agent_tool_audit_events(session_id.as_deref(), limit.unwrap_or(80))
        .map_err(|error| error.to_string())
}

/// P0-4: the structured permission-decision ledger for one run (谁批/为什么/何时).
#[tauri::command]
pub async fn get_agent_permission_decisions(
    run_id: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<PermissionDecisionRecord>, String> {
    state
        .local_db
        .permission_decisions_for_run(&run_id, limit.unwrap_or(200))
        .map_err(|error| error.to_string())
}

/// P1-7: resolve a pending needs_confirm action by writing the user's approve /
/// deny back into the permission-decision ledger (the confirmation event chain).
/// `decided_by` is "user" — distinct from the automated gate/policy/skill layers —
/// and the resolution lands in the run timeline (P1-3) so it is replayable and
/// auditable. The four-element confirmation card UI is deferred (architecture-only
/// scope); this is its backend write-back.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn resolve_permission_confirmation(
    run_id: String,
    iteration: Option<usize>,
    tool_call_id: String,
    subject: String,
    action: String,
    risk: String,
    mode: String,
    approved: bool,
    impact: Option<String>,
    session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<PermissionDecisionRecord, String> {
    // Carry the action's impact / scope (a confirmation-card field) in the reason,
    // so the ledger keeps a human-readable record without a schema migration.
    let verdict = if approved { "批准" } else { "拒绝" };
    let reason = match impact.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(impact) => format!("用户{verdict}危险动作（影响：{impact}）"),
        None => format!("用户{verdict}危险动作"),
    };
    state
        .local_db
        .log_permission_decision(LogPermissionDecisionPayload {
            session_id,
            run_id,
            iteration: iteration.unwrap_or(0),
            tool_call_id,
            subject,
            action,
            risk,
            mode,
            decision: if approved { "allowed" } else { "denied" }.to_string(),
            reason,
            decided_by: "user".to_string(),
        })
        .map_err(|error| error.to_string())
}

/// P1-6: classify a user message into a structured intent (chat / question /
/// task / edit / debug / review) with action and clarification flags. Rule-based
/// and deterministic.
#[tauri::command]
pub async fn classify_user_intent(message: String) -> Result<UserIntent, String> {
    Ok(classify_intent(&message))
}

/// P1-6: scan a project root into a structured snapshot (stack, package managers,
/// key files, entry points, how to test) with dependency / build dirs excluded.
/// Defaults to the process working directory when no path is given.
#[tauri::command]
pub async fn get_project_snapshot(path: Option<String>) -> Result<ProjectSnapshot, String> {
    let root = match path {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => std::env::current_dir().map_err(|error| error.to_string())?,
    };
    Ok(scan_project_snapshot(&root))
}

/// P0-4: derive an independent, structured permission decision from a tool-audit
/// event. Returns `None` for post-execution outcomes (`executed`/`error`), which
/// are not permission decisions.
pub(crate) fn permission_decision_from_audit(
    payload: &LogAgentToolAuditPayload,
) -> Option<LogPermissionDecisionPayload> {
    let decision = match payload.status.as_str() {
        "allowed" => "allowed",
        "blocked" => "denied",
        // executed / error are execution outcomes, not permission decisions.
        _ => return None,
    };
    Some(LogPermissionDecisionPayload {
        session_id: payload.session_id.clone(),
        run_id: payload.run_id.clone(),
        iteration: payload.iteration,
        tool_call_id: payload.tool_call_id.clone(),
        subject: permission_subject_for_tool(&payload.tool_name).to_string(),
        action: payload.tool_name.clone(),
        risk: permission_risk_for_tool(&payload.tool_name).to_string(),
        mode: payload.permission_mode.clone(),
        decision: decision.to_string(),
        reason: payload.reason.clone(),
        decided_by: decided_by_from_reason(&payload.reason).to_string(),
    })
}

/// Which layer made the decision, derived from the audit reason code emitted by
/// `core.rs::execute_tool`.
fn decided_by_from_reason(reason: &str) -> &'static str {
    match reason {
        "tools_disabled" | "policy_denies_all" | "no_active_task" => "gate",
        "skill_blocks_tool" => "skill",
        _ => "policy",
    }
}

/// Coarse subject classification by tool name. The precise risk/safety lives in
/// `ToolMetadata`, which the audit payload doesn't carry; this is a stable,
/// name-based approximation for the decision ledger.
fn permission_subject_for_tool(tool_name: &str) -> &'static str {
    if tool_name == "git" || tool_name.starts_with("git_") {
        "git"
    } else if tool_name.starts_with("plugin_")
        || matches!(
            tool_name,
            "install_plugin_package"
                | "list_plugin_packages"
                | "set_plugin_package_enabled"
                | "invoke_plugin_capability"
        )
    {
        "plugin"
    } else if tool_name.contains("mcp") {
        "mcp"
    } else if matches!(
        tool_name,
        "search_web" | "fetch_web_page" | "open_web_search" | "get_github_trending"
    ) {
        "network"
    } else if matches!(tool_name, "run_command" | "prepare_command") {
        "command"
    } else if matches!(
        tool_name,
        "write_file"
            | "prepare_file_write"
            | "edit_file"
            | "create_directory"
            | "read_file"
            | "list_directory"
            | "search_files"
            | "file_info"
    ) {
        "file"
    } else {
        "other"
    }
}

fn permission_risk_for_tool(tool_name: &str) -> &'static str {
    match tool_name {
        "run_command" | "prepare_command" | "write_file" | "prepare_file_write" | "edit_file"
        | "create_directory" => "destructive",
        "search_web"
        | "fetch_web_page"
        | "open_web_search"
        | "get_github_trending"
        | "invoke_mcp_tool"
        | "install_plugin_package"
        | "set_plugin_package_enabled"
        | "invoke_plugin_capability" => "sensitive",
        _ => "safe",
    }
}

#[tauri::command]
pub async fn retry_agent_run(
    run_id: String,
    instruction: Option<String>,
    state: State<'_, AppState>,
    window: Window,
) -> Result<String, String> {
    let run = state
        .local_db
        .get_agent_run(&run_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "没有找到这次 Agent 运行。".to_string())?;
    if run.status == "running" {
        return Err("这次运行仍在进行中，不能重复继续。".to_string());
    }
    let session_id = run
        .session_id
        .clone()
        .ok_or_else(|| "这次运行没有关联会话，不能自动继续。".to_string())?;
    let original = state
        .local_db
        .get_messages(&session_id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .rfind(|message| message.role == "user" && message.created_at <= run.created_at + 2_000)
        .map(|message| message.content)
        .unwrap_or_else(|| "继续上次任务。".to_string());
    let steps = state
        .local_db
        .get_agent_run_steps(&run.id)
        .map_err(|error| error.to_string())?;
    let step_summary = steps
        .iter()
        .rev()
        .take(8)
        .map(|step| format!("- [{} / {}] {}", step.step_type, step.status, step.summary))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let visible_message = instruction
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("继续上次中断的任务")
        .to_string();
    state
        .local_db
        .save_message(
            &session_id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: visible_message.clone(),
                created_at: None,
                metadata: serde_json::json!({
                    "source": "retry_agent_run",
                    "retryRunId": run.id.clone(),
                    "previousStatus": run.status.clone()
                }),
            },
        )
        .map_err(|error| error.to_string())?;
    let backend_message = format!(
        "[继续上次 Agent 任务]\n上次运行：{}\n上次状态：{}\n上次错误：{}\n\n上次关键步骤：\n{}\n\n原始任务：\n{}\n\n用户补充：\n{}\n\n请从最后一个安全点继续。不要假装已经恢复；如果无法可靠继续，先检查真实文件、命令或工具状态，再说明下一步。",
        run.id,
        run.status,
        run.error.clone().unwrap_or_else(|| "无".to_string()),
        if step_summary.trim().is_empty() { "无记录" } else { &step_summary },
        original,
        visible_message,
    );
    let history = build_conversation_context(&session_id, &state, "chat")?;
    let response = run_agent(
        backend_message,
        history,
        state.clone(),
        window,
        Some(session_id.clone()),
        "chat",
        Vec::new(),
        None,
    )
    .await?;
    state
        .local_db
        .save_message(
            &session_id,
            SaveMessagePayload {
                id: None,
                role: "assistant".to_string(),
                content: response.clone(),
                created_at: None,
                metadata: serde_json::json!({
                    "source": "retry_agent_run",
                    "previousRunId": run.id.clone()
                }),
            },
        )
        .map_err(|error| error.to_string())?;
    Ok(response)
}

fn emit_subagent_event(window: &Window, session_id: &str, event: AgentEvent) {
    window
        .emit(
            "agent-event",
            serde_json::json!({
                "sessionId": session_id,
                "event": event
            }),
        )
        .ok();
}

async fn active_run_for_session(state: &State<'_, AppState>, session_id: &str) -> Option<String> {
    state
        .active_session_runs
        .lock()
        .await
        .get(session_id)
        .cloned()
}

async fn queue_run_guidance(
    state: &State<'_, AppState>,
    run_id: &str,
    message: String,
    attachments: Vec<AgentAttachment>,
) {
    let mut queues = state.run_guidance.lock().await;
    queues
        .entry(run_id.to_string())
        .or_default()
        .push(AgentGuidanceMessage {
            content: message,
            attachments,
        });
}

fn skill_snapshot_for_project_root(
    db: &LocalDb,
    project_root: Option<&Path>,
) -> Result<SkillRegistrySnapshot, String> {
    let states = load_skill_states(db)?;
    let user_dir = user_skills_dir();
    let project_dir = project_root.map(project_skills_dir);
    Ok(load_skill_snapshot(
        Some(&user_dir),
        project_dir.as_deref(),
        &states,
    ))
}

fn should_auto_apply_skills(db: &LocalDb, message: &str) -> bool {
    if message.trim_start().starts_with("使用 Skill「") {
        return true;
    }
    let mode = db
        .get_app_state(AGENT_SKILL_AUTO_MODE_STATE_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(|value| value.to_string()))
        .unwrap_or_else(|| "auto".to_string());
    mode == "auto"
}

fn load_skill_states(db: &LocalDb) -> Result<BTreeMap<String, SkillState>, String> {
    Ok(parse_skill_states(
        db.get_app_state(AGENT_SKILL_STATE_KEY)
            .map_err(|error| error.to_string())?,
    ))
}

fn persist_agent_event(
    db: &LocalDb,
    fallback_run_id: &str,
    session_id: Option<&str>,
    event: &AgentEvent,
) {
    match event {
        AgentEvent::SubAgentStarted {
            subagent_id,
            name,
            task,
            ..
        } => {
            let _ = db.append_agent_run_step(
                fallback_run_id,
                "subagent",
                "running",
                &format!("子代理 {name} 开始处理。"),
                serde_json::json!({ "subagentId": subagent_id, "name": name, "task": task }),
                serde_json::json!({}),
            );
        }
        AgentEvent::SubAgentFinished {
            subagent_id,
            name,
            summary,
        } => {
            let _ = db.append_agent_run_step(
                fallback_run_id,
                "subagent",
                "finished",
                summary,
                serde_json::json!({ "subagentId": subagent_id, "name": name }),
                serde_json::json!({ "summary": summary }),
            );
        }
        AgentEvent::SubAgentFailed {
            subagent_id,
            name,
            error,
        } => {
            let _ = db.append_agent_run_step(
                fallback_run_id,
                "subagent",
                "failed",
                error,
                serde_json::json!({ "subagentId": subagent_id, "name": name }),
                serde_json::json!({ "error": error }),
            );
        }
        AgentEvent::Thinking { content } => {
            if content.trim().is_empty() {
                return;
            }
            let _ = db.append_agent_run_step(
                fallback_run_id,
                "thinking",
                "finished",
                content,
                serde_json::json!({}),
                serde_json::json!({}),
            );
        }
        AgentEvent::ToolCall { tool_call } => {
            let _ = db.append_agent_run_step(
                fallback_run_id,
                "tool_call",
                "running",
                &format!("调用工具：{}", tool_call.name),
                serde_json::json!({
                    "toolCallId": tool_call.id.clone(),
                    "toolName": tool_call.name.clone(),
                    "arguments": tool_call.arguments.clone(),
                }),
                serde_json::json!({}),
            );
        }
        AgentEvent::ToolResult { result } => {
            let mut parsed =
                serde_json::from_str::<serde_json::Value>(result).unwrap_or_else(|_| {
                    serde_json::json!({
                        "raw": result,
                    })
                });
            annotate_tool_error_recovery(&mut parsed);
            let _ = db.finish_latest_agent_tool_call_step(fallback_run_id, parsed.clone());
            let summary = parsed
                .get("summary")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("工具调用完成。")
                .to_string();
            let step_type = if parsed
                .get("data")
                .and_then(|data| data.get("pendingCommand"))
                .is_some()
            {
                persist_pending_command_message(db, fallback_run_id, session_id, result, &parsed);
                "approval"
            } else if parsed
                .get("data")
                .and_then(|data| data.get("commandResult"))
                .is_some()
            {
                "command"
            } else if parsed
                .get("data")
                .and_then(|data| data.get("path"))
                .and_then(|value| value.as_str())
                .is_some()
            {
                "file_change"
            } else {
                "tool_result"
            };
            if matches!(step_type, "file_change" | "tool_result") {
                record_file_artifact_from_tool_result(
                    db,
                    fallback_run_id,
                    session_id,
                    &summary,
                    &parsed,
                );
            }
            let _ = db.append_agent_run_step(
                fallback_run_id,
                step_type,
                "finished",
                &summary,
                serde_json::json!({}),
                parsed,
            );
        }
        AgentEvent::OperationPreparing {
            label,
            detail,
            tool_name,
            bytes,
        }
        | AgentEvent::OperationProgress {
            label,
            detail,
            tool_name,
            bytes,
        } => {
            let summary = detail
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|detail| format!("{label}：{detail}"))
                .unwrap_or_else(|| label.clone());
            let _ = db.append_agent_run_step(
                fallback_run_id,
                "operation",
                "finished",
                &summary,
                serde_json::json!({
                    "toolName": tool_name,
                    "bytes": bytes,
                }),
                serde_json::json!({}),
            );
        }
        AgentEvent::OperationStarted {
            operation_id,
            tool_name,
            label,
            detail,
            target,
            command,
        } => {
            let _ = db.append_agent_run_step(
                fallback_run_id,
                "operation",
                "finished",
                detail.as_deref().unwrap_or(label),
                serde_json::json!({
                    "operationId": operation_id,
                    "toolName": tool_name,
                    "label": label,
                    "target": target,
                    "command": command,
                }),
                serde_json::json!({}),
            );
        }
        AgentEvent::OperationOutput {
            operation_id: _,
            stream: _,
            content: _,
        } => {}
        AgentEvent::OperationFinished {
            operation_id,
            status,
            summary,
        } => {
            let _ = db.append_agent_run_step(
                fallback_run_id,
                "operation",
                "finished",
                summary,
                serde_json::json!({
                    "operationId": operation_id,
                    "status": status,
                }),
                serde_json::json!({}),
            );
        }
        AgentEvent::OperationFailed {
            operation_id,
            summary,
        } => {
            let _ = db.append_agent_run_step(
                fallback_run_id,
                "operation",
                "failed",
                summary,
                serde_json::json!({
                    "operationId": operation_id,
                }),
                serde_json::json!({}),
            );
        }
        AgentEvent::RunEvent { event } => match event {
            AgentRunEvent::Started { .. } => {
                let _ = db.update_agent_run_status(fallback_run_id, "running", None);
            }
            AgentRunEvent::Iteration { run_id, iteration } => {
                let _ = db.append_agent_run_step(
                    run_id,
                    "iteration",
                    "finished",
                    &format!("开始第 {iteration} 轮。"),
                    serde_json::json!({ "iteration": iteration }),
                    serde_json::json!({}),
                );
            }
            AgentRunEvent::GuidanceMerged { run_id, count } => {
                let _ = db.append_agent_run_step(
                    run_id,
                    "guidance",
                    "finished",
                    &format!("已纳入 {count} 条用户补充消息。"),
                    serde_json::json!({ "count": count }),
                    serde_json::json!({}),
                );
            }
            AgentRunEvent::GuidanceQueued { run_id, count } => {
                let _ = db.append_agent_run_step(
                    run_id,
                    "guidance",
                    "finished",
                    &format!("已排队 {count} 条用户补充消息。"),
                    serde_json::json!({ "count": count }),
                    serde_json::json!({ "queued": true }),
                );
            }
            AgentRunEvent::Finished { run_id } => {
                let _ = db.update_agent_run_status(run_id, "finished", None);
            }
            AgentRunEvent::Blocked {
                run_id,
                status,
                footer,
            } => {
                let source = if status == "waiting_confirmation" {
                    "token_budget"
                } else {
                    "final_audit"
                };
                let _ = db.update_agent_run_status(
                    run_id,
                    "blocked",
                    Some(&format!("{source}={status}: {footer}")),
                );
            }
            AgentRunEvent::Paused { run_id } => {
                let _ = db.update_agent_run_status(run_id, "paused", None);
            }
            AgentRunEvent::Resumed { run_id } => {
                let _ = db.update_agent_run_status(run_id, "running", None);
            }
            AgentRunEvent::Cancelled { run_id } => {
                let _ = db.update_agent_run_status(run_id, "cancelled", Some("用户取消任务。"));
            }
            AgentRunEvent::Failed { run_id, error, .. } => {
                let _ = db.update_agent_run_status(run_id, "failed", Some(error));
            }
            AgentRunEvent::ToolResult { .. } => {}
        },
        AgentEvent::ResponseCompleted { content, .. } | AgentEvent::Response { content, .. } => {
            if !content.trim().is_empty() {
                let _ = db.append_agent_run_step(
                    fallback_run_id,
                    "response",
                    "finished",
                    "生成最终回复。",
                    serde_json::json!({}),
                    serde_json::json!({ "content": content }),
                );
            }
        }
        AgentEvent::ResponseStarted { .. }
        | AgentEvent::ResponseDelta { .. }
        | AgentEvent::ResponseFallbackStarted { .. }
        | AgentEvent::FinalAudit { .. } => {}
    }
}

fn persist_pending_command_message(
    db: &LocalDb,
    run_id: &str,
    session_id: Option<&str>,
    content: &str,
    parsed: &serde_json::Value,
) {
    let Some(session_id) = session_id else {
        return;
    };
    let Some(pending_id) = parsed
        .get("data")
        .and_then(|data| data.get("pendingCommand"))
        .and_then(|pending| pending.get("id"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let _ = db.save_message(
        session_id,
        SaveMessagePayload {
            id: Some(pending_command_message_id(pending_id)),
            role: "tool".to_string(),
            content: content.to_string(),
            created_at: None,
            metadata: serde_json::json!({
                "source": "agent_pending_command",
                "runId": run_id,
                "pendingCommandId": pending_id
            }),
        },
    );
}

fn record_file_artifact_from_tool_result(
    db: &LocalDb,
    run_id: &str,
    session_id: Option<&str>,
    summary: &str,
    parsed: &serde_json::Value,
) {
    let data = parsed.get("data").unwrap_or(parsed);
    let candidate = data
        .get("fileWrite")
        .or_else(|| data.get("fileEdit"))
        .or_else(|| data.get("pendingWrite"));
    let Some(file) = candidate else {
        return;
    };
    let Some(path) = file
        .get("targetPath")
        .or_else(|| file.get("target_path"))
        .and_then(|value| value.as_str())
    else {
        return;
    };
    let operation = file
        .get("operation")
        .and_then(|value| value.as_str())
        .unwrap_or("write");
    let title = std::path::Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("文件")
        .to_string();
    let status = if data
        .get("confirmed")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        "written"
    } else {
        "pending"
    };
    let _ = db.record_artifact(RecordArtifactPayload {
        session_id: session_id.map(str::to_string),
        run_id: Some(run_id.to_string()),
        kind: "file".to_string(),
        title,
        path: Some(path.to_string()),
        operation: operation.to_string(),
        status: status.to_string(),
        summary: summary.to_string(),
        metadata: serde_json::json!({
            "toolResult": parsed,
            "untrustedExternal": false
        }),
    });
}

fn annotate_tool_error_recovery(value: &mut serde_json::Value) {
    let status = value
        .get("status")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if status != "error" {
        return;
    }
    let summary = value
        .get("summary")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .to_string();
    let lower = summary.to_ascii_lowercase();
    let (kind, hint, recoverable) = if lower.contains("permission")
        || summary.contains("权限")
        || summary.contains("确认")
        || summary.contains("拒绝")
    {
        (
            "permission",
            "检查当前权限模式，必要时让用户确认或切换到默认/完全访问。",
            true,
        )
    } else if lower.contains("not found")
        || summary.contains("找不到")
        || summary.contains("路径")
        || summary.contains("不存在")
    {
        (
            "path",
            "检查目标路径是否存在、是否在允许范围内，再重试。",
            true,
        )
    } else if lower.contains("timeout")
        || lower.contains("connection")
        || summary.contains("超时")
        || summary.contains("网络")
        || summary.contains("连接")
    {
        ("network", "检查网络或本地服务状态，稍后重试。", true)
    } else if lower.contains("model")
        || lower.contains("api key")
        || summary.contains("模型")
        || summary.contains("密钥")
    {
        ("model", "检查模型配置、密钥、URL 和模型名。", true)
    } else {
        (
            "unknown",
            "查看工具输出和运行轨迹，确认失败原因后再继续。",
            false,
        )
    };
    if let Some(object) = value.as_object_mut() {
        object.insert("errorKind".to_string(), serde_json::json!(kind));
        object.insert("recoveryHint".to_string(), serde_json::json!(hint));
        object.insert("recoverable".to_string(), serde_json::json!(recoverable));
    }
}

pub(crate) fn pending_command_message_id(pending_command_id: &str) -> String {
    format!("tool_msg_{pending_command_id}")
}

fn build_conversation_context(
    session_id: &str,
    state: &State<'_, AppState>,
    agent_mode: &str,
) -> Result<Vec<Message>, String> {
    build_conversation_context_from_db(session_id, &state.local_db, agent_mode)
}

fn build_conversation_context_for_current_message(
    session_id: &str,
    state: &State<'_, AppState>,
    agent_mode: &str,
    current_message: &str,
) -> Result<Vec<Message>, String> {
    let mut context = build_conversation_context_from_db_with_history_mode(
        session_id,
        &state.local_db,
        agent_mode,
        if user_message_should_use_conversation_history(current_message) {
            ConversationHistoryMode::Full
        } else {
            ConversationHistoryMode::CurrentMessageOnly
        },
    )?;
    if memory_injection_enabled(&state.local_db) && !current_message.trim().is_empty() {
        let (retrieval, feedback) = recall_knowledge_with_feedback(
            &state.local_db,
            KnowledgeRecallRequest {
                query: current_message.to_string(),
                scope: Some(format!("session:{session_id}")),
                limit: Some(5),
            },
        )?;
        if let Some(note) = retrieval.system_note {
            context.push(Message::plain(Role::System, note));
        }
        if !feedback.reinforced_item_ids.is_empty() || !feedback.decayed_item_ids.is_empty() {
            context.push(Message::plain(
                Role::System,
                format!(
                    "[长期知识相关性反馈] 本轮仅强化检索命中的记忆 {} 条，衰减未命中的同 scope/global 记忆 {} 条；这只是记忆质量信号，不是任务完成证据。",
                    feedback.reinforced_item_ids.len(),
                    feedback.decayed_item_ids.len() + feedback.soft_deleted_item_ids.len()
                ),
            ));
        }
    }
    Ok(context)
}

fn context_usage_snapshot_for_message(
    db: &LocalDb,
    session_id: &str,
    agent_mode: &str,
    current_message: &str,
    attachments: Vec<AgentAttachment>,
) -> Result<ContextUsageSnapshot, String> {
    let history_mode = if user_message_should_use_conversation_history(current_message) {
        ConversationHistoryMode::Full
    } else {
        ConversationHistoryMode::CurrentMessageOnly
    };
    let history = build_conversation_context_from_db_with_history_mode(
        session_id,
        db,
        agent_mode,
        history_mode,
    )?;
    let summary_included = context_has_summary(&history);
    let prompt_messages = crate::agent::ContextBuilder::build_with_skill_prompt(
        current_message.to_string(),
        history,
        None,
        attachments,
    );
    let used_tokens = estimate_context_window_tokens(&prompt_messages);
    let message_count = db.get_messages(session_id).map_err(|e| e.to_string())?.len();
    let compression_state = if summary_included {
        "compressed"
    } else if used_tokens >= ATLAS_CONTEXT_COMPRESSION_TRIGGER_TOKENS {
        "near_limit"
    } else {
        "ok"
    };
    Ok(ContextUsageSnapshot {
        session_id: Some(session_id.to_string()),
        used_tokens,
        limit_tokens: ATLAS_CONTEXT_TOKEN_LIMIT,
        ratio: used_tokens as f64 / ATLAS_CONTEXT_TOKEN_LIMIT as f64,
        source: "estimated".to_string(),
        compression_state: compression_state.to_string(),
        summary_included,
        message_count,
        updated_at: command_now_ms(),
    })
}

fn ensure_context_window_compressed_for_send(
    db: &LocalDb,
    session_id: &str,
    agent_mode: &str,
    current_message: &str,
    attachments: Vec<AgentAttachment>,
) -> Result<(), String> {
    let snapshot =
        context_usage_snapshot_for_message(db, session_id, agent_mode, current_message, attachments)?;
    if snapshot.used_tokens >= ATLAS_CONTEXT_COMPRESSION_TRIGGER_TOKENS {
        db.summarize_session(session_id).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn context_has_summary(messages: &[Message]) -> bool {
    messages.iter().any(|message| {
        message
            .content
            .contains("summaryIncluded=true")
            || message.content.starts_with("以下是本会话较早内容的压缩摘要")
    })
}

fn estimate_context_window_tokens(messages: &[Message]) -> usize {
    let chars = messages.iter().map(estimated_message_chars).sum::<usize>();
    chars.div_ceil(CONTEXT_TOKEN_ESTIMATE_CHARS_PER_TOKEN)
}

fn estimated_message_chars(message: &Message) -> usize {
    message.model_content().chars().count()
        + message
            .attachments
            .iter()
            .map(estimated_attachment_chars)
            .sum::<usize>()
}

fn estimated_attachment_chars(attachment: &AgentAttachment) -> usize {
    attachment.name.chars().count()
        + attachment.mime.chars().count()
        + attachment
            .text_preview
            .as_deref()
            .map(|value| value.chars().count())
            .unwrap_or(0)
        + attachment
            .data_url
            .as_deref()
            .map(|_| attachment.size / 3)
            .unwrap_or(0)
}

fn command_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn build_conversation_context_from_db(
    session_id: &str,
    db: &LocalDb,
    agent_mode: &str,
) -> Result<Vec<Message>, String> {
    build_conversation_context_from_db_with_history_mode(
        session_id,
        db,
        agent_mode,
        ConversationHistoryMode::Full,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConversationHistoryMode {
    Full,
    CurrentMessageOnly,
}

impl ConversationHistoryMode {
    fn label(self) -> &'static str {
        match self {
            ConversationHistoryMode::Full => "full",
            ConversationHistoryMode::CurrentMessageOnly => "current_message_only",
        }
    }
}

struct SessionContextWindow {
    messages: Vec<Message>,
}

#[derive(Clone, Copy)]
struct ContextCompressionPolicy {
    recent_message_limit: usize,
    summary_trigger_message_count: usize,
    protected_recent_message_floor: usize,
    soft_char_budget: usize,
}

impl Default for ContextCompressionPolicy {
    fn default() -> Self {
        Self {
            recent_message_limit: CONTEXT_WINDOW_RECENT_MESSAGE_LIMIT,
            summary_trigger_message_count: CONTEXT_WINDOW_SUMMARY_TRIGGER_MESSAGE_COUNT,
            protected_recent_message_floor: CONTEXT_WINDOW_PROTECTED_RECENT_MESSAGE_FLOOR,
            soft_char_budget: CONTEXT_WINDOW_SOFT_CHAR_BUDGET,
        }
    }
}

struct RecentContextSelection {
    messages: Vec<MessageRecord>,
    selected_char_count: usize,
    budget_exceeded: bool,
    pruned_for_budget: bool,
    floor_preserved_over_budget: bool,
}

struct ContextWindowAudit {
    session_id: String,
    history_mode: ConversationHistoryMode,
    durable_user_assistant_messages: usize,
    selected_recent_messages: usize,
    omitted_recent_messages: usize,
    summary_included: bool,
    clear_after_applied: bool,
    plan_anchor_pinned: bool,
    active_task_pinned: bool,
    compression_triggered: bool,
    compression_reason: String,
    context_soft_char_budget: usize,
    selected_recent_chars: usize,
    pinned_task_anchor_chars: usize,
    protected_recent_message_floor: usize,
}

impl ContextWindowAudit {
    fn system_note(&self) -> String {
        format!(
            "[ContextWindow 选择策略]\nsource=persistent_session\nsessionId={}\nhistoryMode={}\ndurableUserAssistantMessages={}\nselectedRecentMessages={}\nomittedOlderMessages={}\nsummaryIncluded={}\nclearAfterApplied={}\nplanAnchorPinned={}\nactiveTaskPinned={}\ncompressionTriggered={}\ncompressionReason={}\ncontextSoftCharBudget={}\nselectedRecentChars={}\npinnedTaskAnchorChars={}\nprotectedRecentMessageFloor={}\n说明：Session/EventLog 是持久事实源；ContextWindow 只是本次模型调用从持久会话中临时选出的窗口，不代表全部记忆。压缩策略只能压旧消息；当前 run/task/goal 锚点在压缩后重新 pin 入窗口，不参与旧消息裁剪。",
            self.session_id,
            self.history_mode.label(),
            self.durable_user_assistant_messages,
            self.selected_recent_messages,
            self.omitted_recent_messages,
            self.summary_included,
            self.clear_after_applied,
            self.plan_anchor_pinned,
            self.active_task_pinned,
            self.compression_triggered,
            self.compression_reason,
            self.context_soft_char_budget,
            self.selected_recent_chars,
            self.pinned_task_anchor_chars,
            self.protected_recent_message_floor,
        )
    }
}

fn build_conversation_context_from_db_with_history_mode(
    session_id: &str,
    db: &LocalDb,
    agent_mode: &str,
    history_mode: ConversationHistoryMode,
) -> Result<Vec<Message>, String> {
    Ok(build_session_context_window_from_db_with_history_mode(
        session_id,
        db,
        agent_mode,
        history_mode,
    )?
    .messages)
}

fn build_session_context_window_from_db_with_history_mode(
    session_id: &str,
    db: &LocalDb,
    agent_mode: &str,
    history_mode: ConversationHistoryMode,
) -> Result<SessionContextWindow, String> {
    let mut context = Vec::new();

    context.push(Message::plain(
        Role::System,
        "你是 Atlas，一个本地桌面 Agent。请用自然、清晰、简洁的中文和用户协作。当前 Agent 核心能力是：读取/搜索本地文件，创建/编辑文本、代码和网页文件，创建目录，以及按权限运行本地命令。用户明确要求创建文件、网页、脚本、样式或安装依赖时，如果当前权限允许，就直接使用工具执行；不要反复要求确认，不要把 .html 写成 .txt 后再让用户手动改名。工具失败时直接说明失败原因和下一步，不要假装成功。普通聊天保持简短；执行任务时先用一句话说明将做什么，然后开始做。".to_string(),
    ));

    if agent_mode == "plan" {
        context.push(Message::plain(
            Role::System,
            "当前是计划模式：只能读取必要信息并整理可执行方案，不要写文件、不要运行命令。最后输出一份等待用户确认的计划，包含目标、步骤、风险和验收标准；不要要求用户手动切换模式，前端会在用户确认后切回普通对话并执行。".to_string(),
        ));
    } else if agent_mode == "code_review" {
        context.push(Message::plain(
            Role::System,
            "当前是代码审查模式：只允许读取和搜索必要的项目文件；不要写文件、不要运行命令、不要输出等待执行的计划。按照代码审查规则 findings first 输出，缺少 diff、测试、CI 或真实验证时标为信息缺失或未验证。".to_string(),
        ));
    }

    let mode = agent_permission_mode(db);
    let policy = tool_access_policy_for(agent_mode, &mode);
    if mode == "plan" && agent_mode != "code_review" {
        context.push(Message::plain(
            Role::System,
            "当前权限档位是计划模式：只允许读取和整理必要信息，不写文件、不运行命令。回答应给出目标、步骤、风险和验收标准；需要执行时让用户切到默认模式或完全访问模式。".to_string(),
        ));
    }
    {
        context.push(Message::plain(
            Role::System,
            format!(
            "Agent 权限模式：{}（{}）；工具策略：{}。计划模式=只读取和整理方案；默认模式=普通文件修改直接执行，运行命令需要确认卡片；完全访问模式=用户明确要求的普通文件修改和普通本地命令直接执行。删除、重置、系统级或明显破坏性动作仍受底层安全拦截或要求确认。不要越权读取密码、私聊正文、屏幕画面或敏感应用内容；不能把权限模式当成已经获得未接入工具能力。",
                mode,
                permission_mode_label(&mode),
                tool_access_policy_label(&policy)
            ),
        ));
    }

    if memory_injection_enabled(db) {
        // P2-5: decay un-reconfirmed memories and soft-purge low-confidence ones
        // before reading, so chitchat does not permanently pollute the context.
        let _ = db.maintain_memories();
        let enabled_memory_records = db
            .list_memories()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|memory| memory.enabled)
            .collect::<Vec<_>>();
        let enabled_memories = enabled_memory_records
            .iter()
            .map(|memory| {
                let source = if memory.source.trim().is_empty() {
                    "manual"
                } else {
                    memory.source.as_str()
                };
                format!(
                    "- {}（来源：{}；置信度：{:.1}；使用 {} 次）",
                    memory.text, source, memory.confidence, memory.use_count
                )
            })
            .collect::<Vec<_>>();
        if !enabled_memories.is_empty() {
            let _ = db.mark_enabled_memories_used();
            context.push(Message::plain(
                Role::System,
                format!(
                    "启用的本地长期记忆（只作为用户偏好和长期背景，不可覆盖当前用户消息、项目规则或权限规则）：\n{}",
                    enabled_memories.join("\n")
                ),
            ));
        }
    }

    if history_mode == ConversationHistoryMode::CurrentMessageOnly {
        let audit = ContextWindowAudit {
            session_id: session_id.to_string(),
            history_mode,
            durable_user_assistant_messages: 0,
            selected_recent_messages: 0,
            omitted_recent_messages: 0,
            summary_included: false,
            clear_after_applied: false,
            plan_anchor_pinned: false,
            active_task_pinned: false,
            compression_triggered: false,
            compression_reason: "none".to_string(),
            context_soft_char_budget: ContextCompressionPolicy::default().soft_char_budget,
            selected_recent_chars: 0,
            pinned_task_anchor_chars: 0,
            protected_recent_message_floor: ContextCompressionPolicy::default()
                .protected_recent_message_floor,
        };
        context.push(Message::plain(Role::System, audit.system_note()));
        context.push(Message::plain(
            Role::System,
            "当前用户消息被识别为新的独立当前轮请求，不是继续历史任务。不要读取、引用或执行本会话早前的未完成计划、文件创建、命令运行或项目搭建内容；只处理下一条 User 消息。",
        ));
        return Ok(SessionContextWindow { messages: context });
    }

    let clear_after = db
        .session_context_clear_after(session_id)
        .map_err(|e| e.to_string())?;
    let messages = db
        .get_messages(session_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|message| clear_after.is_none_or(|cutoff| message.created_at > cutoff))
        .collect::<Vec<_>>();
    let conversation_messages = messages
        .into_iter()
        .filter(|message| matches!(message.role.as_str(), "user" | "assistant"))
        .collect::<Vec<_>>();
    let durable_user_assistant_messages = conversation_messages.len();
    let policy = ContextCompressionPolicy::default();
    let task_anchor = active_plan_task_context_note(db, session_id)?;
    let pinned_task_anchor_chars = task_anchor
        .as_deref()
        .map(context_note_char_count)
        .unwrap_or(0);
    let base_reserved_chars = context_messages_char_count(&context)
        + pinned_task_anchor_chars
        + CONTEXT_WINDOW_AUDIT_NOTE_RESERVED_CHARS;
    let preview_selection =
        select_recent_context_messages(&conversation_messages, policy, base_reserved_chars);
    let count_requires_summary =
        durable_user_assistant_messages > policy.summary_trigger_message_count;
    let budget_requires_summary = preview_selection.budget_exceeded;
    let summary_included =
        clear_after.is_none() && (count_requires_summary || budget_requires_summary);
    if summary_included {
        let summary = db
            .get_session_summary(session_id)
            .map_err(|e| e.to_string())?
            .map(Ok)
            .unwrap_or_else(|| db.summarize_session(session_id))
            .map_err(|e| e.to_string())?;
        context.push(Message::plain(Role::System, summary.summary));
    }

    let plan_anchor_pinned = task_anchor
        .as_deref()
        .is_some_and(|note| note.contains("planId="));
    let active_task_pinned = task_anchor
        .as_deref()
        .is_some_and(|note| note.contains("activeTaskId="));
    let reserved_chars = context_messages_char_count(&context)
        + pinned_task_anchor_chars
        + CONTEXT_WINDOW_AUDIT_NOTE_RESERVED_CHARS;
    let selected_recent =
        select_recent_context_messages(&conversation_messages, policy, reserved_chars);
    let mut compression_reasons = Vec::new();
    if count_requires_summary {
        compression_reasons.push(format!(
            "message_count>{}",
            policy.summary_trigger_message_count
        ));
    }
    if budget_requires_summary {
        compression_reasons.push(format!("soft_char_budget>{}", policy.soft_char_budget));
    }
    if selected_recent.pruned_for_budget {
        compression_reasons.push("recent_tail_pruned_to_budget".to_string());
    }
    if selected_recent.floor_preserved_over_budget {
        compression_reasons.push("protected_recent_floor_preserved".to_string());
    }
    if summary_included {
        compression_reasons.push("summary_included".to_string());
    }
    let compression_triggered = clear_after.is_none() && !compression_reasons.is_empty();
    let audit = ContextWindowAudit {
        session_id: session_id.to_string(),
        history_mode,
        durable_user_assistant_messages,
        selected_recent_messages: selected_recent.messages.len(),
        omitted_recent_messages: durable_user_assistant_messages
            .saturating_sub(selected_recent.messages.len()),
        summary_included,
        clear_after_applied: clear_after.is_some(),
        plan_anchor_pinned,
        active_task_pinned,
        compression_triggered,
        compression_reason: if compression_triggered {
            compression_reasons.join("+")
        } else {
            "none".to_string()
        },
        context_soft_char_budget: policy.soft_char_budget,
        selected_recent_chars: selected_recent.selected_char_count,
        pinned_task_anchor_chars,
        protected_recent_message_floor: policy.protected_recent_message_floor,
    };
    context.push(Message::plain(Role::System, audit.system_note()));
    if let Some(task_anchor) = task_anchor {
        context.push(Message::plain(Role::System, task_anchor));
    }

    for message in selected_recent.messages {
        let attachments =
            drop_historical_image_attachments(attachments_from_message_metadata(&message.metadata));
        context.push(Message::with_attachments(
            if message.role == "user" {
                Role::User
            } else {
                Role::Assistant
            },
            message.content,
            attachments,
        ));
    }

    Ok(SessionContextWindow { messages: context })
}

fn select_recent_context_messages(
    messages: &[MessageRecord],
    policy: ContextCompressionPolicy,
    reserved_chars: usize,
) -> RecentContextSelection {
    let mut selected = messages
        .iter()
        .rev()
        .take(policy.recent_message_limit)
        .cloned()
        .collect::<Vec<_>>();
    selected.reverse();

    let mut selected_char_count = message_records_char_count(&selected);
    let budget_exceeded = reserved_chars + selected_char_count > policy.soft_char_budget;
    let mut pruned_for_budget = false;
    if budget_exceeded {
        let protected_floor = policy
            .protected_recent_message_floor
            .min(policy.recent_message_limit)
            .min(selected.len());
        while selected.len() > protected_floor
            && reserved_chars + selected_char_count > policy.soft_char_budget
        {
            let removed = selected.remove(0);
            selected_char_count =
                selected_char_count.saturating_sub(message_record_char_count(&removed));
            pruned_for_budget = true;
        }
    }

    let floor_preserved_over_budget =
        budget_exceeded && reserved_chars + selected_char_count > policy.soft_char_budget;
    RecentContextSelection {
        messages: selected,
        selected_char_count,
        budget_exceeded,
        pruned_for_budget,
        floor_preserved_over_budget,
    }
}

fn context_messages_char_count(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|message| context_note_char_count(&message.content))
        .sum()
}

fn message_records_char_count(messages: &[MessageRecord]) -> usize {
    messages.iter().map(message_record_char_count).sum()
}

fn message_record_char_count(message: &MessageRecord) -> usize {
    context_note_char_count(&message.content)
}

fn context_note_char_count(value: &str) -> usize {
    value.chars().count()
}

fn active_plan_task_context_note(db: &LocalDb, session_id: &str) -> Result<Option<String>, String> {
    let latest_plan = db
        .list_run_plans(session_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .next();
    let active_task = db
        .get_active_plan_task(session_id)
        .map_err(|e| e.to_string())?;

    if latest_plan.is_none() && active_task.is_none() {
        return Ok(None);
    }

    let mut lines = vec![
        "[ContextWindow 当前持久任务锚点]".to_string(),
        "来源=run_plans/plan_tasks 持久表；该锚点是临时注入，不写回 Session。".to_string(),
    ];
    if let Some(plan) = latest_plan {
        lines.push(format!("planId={}", plan.id));
        lines.push(format!("planStatus={}", plan.status));
        lines.push(format!("goal={}", pinned_context_text(&plan.goal)));
        if let Some(observable) = plan.observable_outcome {
            lines.push(format!(
                "observableOutcome={}",
                pinned_context_text(&observable)
            ));
        }
    }
    if let Some(task) = active_task {
        lines.push(format!("activeTaskId={}", task.id));
        lines.push(format!(
            "activeTaskTitle={}",
            pinned_context_text(&task.title)
        ));
        lines.push(format!("activeTaskStatus={}", task.status));
        lines.push(format!("activeTaskEvidenceStatus={}", task.evidence_status));
        if let Some(reason) = task.blocked_reason {
            lines.push(format!(
                "activeTaskBlockedReason={}",
                pinned_context_text(&reason)
            ));
        }
    }

    Ok(Some(lines.join("\n")))
}

fn pinned_context_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_agent_mode(mode: Option<String>) -> String {
    match mode.as_deref() {
        Some("plan") => "plan".to_string(),
        Some("code_review") | Some("review") => "code_review".to_string(),
        _ => "chat".to_string(),
    }
}

fn agent_permission_mode(db: &LocalDb) -> String {
    let raw = db
        .get_app_state("agent_permission_mode")
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_string));
    let mode = AgentPermissionMode::normalize(raw.as_deref());
    let normalized = mode.as_str().to_string();
    if raw.as_deref() != Some(normalized.as_str()) {
        let _ = db.set_app_state(
            "agent_permission_mode",
            serde_json::json!(normalized.clone()),
        );
    }
    normalized
}

fn attachments_from_message_metadata(metadata: &serde_json::Value) -> Vec<AgentAttachment> {
    metadata
        .get("attachments")
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<AgentAttachment>>(value).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|mut attachment| {
            if attachment.kind.trim().is_empty() {
                attachment.kind = if attachment.mime.to_lowercase().starts_with("image/") {
                    "image".to_string()
                } else if attachment.text_preview.is_some() {
                    "text".to_string()
                } else {
                    "file".to_string()
                };
            }
            attachment
        })
        .collect()
}

fn strip_historical_image_attachments(mut message: Message) -> Message {
    message.attachments = drop_historical_image_attachments(message.attachments);
    message
}

fn drop_historical_image_attachments(attachments: Vec<AgentAttachment>) -> Vec<AgentAttachment> {
    attachments
        .into_iter()
        .filter(|attachment| !is_image_attachment(attachment))
        .collect()
}

fn is_image_attachment(attachment: &AgentAttachment) -> bool {
    attachment.kind.eq_ignore_ascii_case("image")
        || attachment.mime.to_lowercase().starts_with("image/")
        || attachment
            .data_url
            .as_deref()
            .map(|value| value.starts_with("data:image/"))
            .unwrap_or(false)
}

fn tool_access_policy_for(agent_mode: &str, permission_mode: &str) -> ToolAccessPolicy {
    let mut mode = AgentPermissionMode::normalize(Some(permission_mode));
    if agent_mode == "plan" || agent_mode == "code_review" {
        mode = AgentPermissionMode::Plan;
    }
    ToolAccessPolicy::from_permission_mode(mode)
}

fn tool_access_policy_label(policy: &ToolAccessPolicy) -> &'static str {
    match policy {
        ToolAccessPolicy::FullAccess => "普通文件和普通命令可直接执行",
        ToolAccessPolicy::Default => "文件修改可执行，命令需要确认",
        ToolAccessPolicy::Plan => "只允许读取和整理方案",
        ToolAccessPolicy::DenyAll => "禁止所有工具",
    }
}

fn permission_mode_label(mode: &str) -> &'static str {
    AgentPermissionMode::normalize(Some(mode)).label_zh()
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelAgentChatResult {
    pub cancelled_count: usize,
    pub session_id: Option<String>,
    pub scope: String,
}

#[tauri::command]
pub async fn cancel_agent_chat(
    session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<CancelAgentChatResult, String> {
    let requested_session_id = session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let scoped_to_session = session_id.is_some();
    let tokens = {
        let mut active = state.cancel_tokens.lock().await;
        if scoped_to_session {
            active
                .remove(&cancel_key_for(requested_session_id.as_deref()))
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            active.drain().map(|(_, token)| token).collect::<Vec<_>>()
        }
    };

    let cancelled_count = tokens.len();
    for token in tokens {
        token.cancel();
    }
    Ok(CancelAgentChatResult {
        cancelled_count,
        session_id: requested_session_id,
        scope: if scoped_to_session {
            "session".to_string()
        } else {
            "all".to_string()
        },
    })
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseAgentChatResult {
    /// 被作用的 run;若当前会话没有活跃 run 则为 None。
    pub run_id: Option<String>,
    /// 是否真的发生状态翻转(幂等:重复 pause / resume 不再翻转,也不重复落库 / 发事件)。
    pub changed: bool,
    /// 调用后的目标状态:"paused" 或 "running"。
    pub status: String,
}

/// P1-2: 暂停当前会话正在运行的 agent。暂停在**工具边界**生效——core 循环跑完
/// 当前安全单元后,在下一次模型调用前挂起,不丢当前 task(取消仍可在暂停期间由
/// `cancel_agent_chat` 从外部 abort)。
#[tauri::command]
pub async fn pause_agent_chat(
    session_id: Option<String>,
    state: State<'_, AppState>,
    window: Window,
) -> Result<PauseAgentChatResult, String> {
    set_run_pause_state(&state, &window, session_id, true).await
}

/// P1-2: 恢复被暂停的 agent,从断点继续。
#[tauri::command]
pub async fn resume_agent_chat(
    session_id: Option<String>,
    state: State<'_, AppState>,
    window: Window,
) -> Result<PauseAgentChatResult, String> {
    set_run_pause_state(&state, &window, session_id, false).await
}

async fn set_run_pause_state(
    state: &State<'_, AppState>,
    window: &Window,
    session_id: Option<String>,
    pause: bool,
) -> Result<PauseAgentChatResult, String> {
    let target_status = if pause { "paused" } else { "running" };
    let trimmed_session = session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    // 经现成的 session→run_id 映射定位活跃 run。未带 session 时,只在全局恰好
    // 一个活跃 run 的情况下作用,避免误伤其他会话。
    let run_id = {
        let active = state.active_session_runs.lock().await;
        match trimmed_session.as_deref() {
            Some(sid) => active.get(sid).cloned(),
            None if active.len() == 1 => active.values().next().cloned(),
            None => None,
        }
    };
    let Some(run_id) = run_id else {
        return Ok(PauseAgentChatResult {
            run_id: None,
            changed: false,
            status: target_status.to_string(),
        });
    };

    let handle = {
        let registry = state.pause_registry.lock().await;
        registry.get(&run_id).cloned()
    };
    let Some(handle) = handle else {
        return Ok(PauseAgentChatResult {
            run_id: Some(run_id),
            changed: false,
            status: target_status.to_string(),
        });
    };

    let changed = if pause {
        handle.pause()
    } else {
        handle.resume()
    };
    if changed {
        // 落库 run 状态;persist_agent_event 对随后 emit 的事件也会落同一状态,双路径幂等。
        let _ = state
            .local_db
            .update_agent_run_status(&run_id, target_status, None);
        let event = if pause {
            AgentRunEvent::Paused {
                run_id: run_id.clone(),
            }
        } else {
            AgentRunEvent::Resumed {
                run_id: run_id.clone(),
            }
        };
        let _ = window.emit(
            "agent-event",
            serde_json::json!({
                "sessionId": trimmed_session,
                "runId": run_id,
                "event": AgentEvent::RunEvent { event },
            }),
        );
    }

    Ok(PauseAgentChatResult {
        run_id: Some(run_id),
        changed,
        status: target_status.to_string(),
    })
}

fn cancel_key_for(session_id: Option<&str>) -> String {
    session_id
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "__legacy_agent_chat__".to_string())
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
        LocalDb::open(std::env::temp_dir().join(format!("aura_agent_context_{unique}.db"))).unwrap()
    }

    fn audit_payload(status: &str, reason: &str, tool_name: &str) -> LogAgentToolAuditPayload {
        LogAgentToolAuditPayload {
            session_id: Some("s".to_string()),
            run_id: "r".to_string(),
            iteration: 0,
            tool_call_id: "t".to_string(),
            tool_name: tool_name.to_string(),
            permission_mode: "default".to_string(),
            policy: "default".to_string(),
            status: status.to_string(),
            reason: reason.to_string(),
        }
    }

    fn route_connection(
        id: &str,
        provider: &str,
        model: &str,
        protocol: &str,
    ) -> crate::config::ModelConnectionConfig {
        crate::config::ModelConnectionConfig {
            id: id.to_string(),
            name: id.to_string(),
            provider_id: provider.to_string(),
            route_id: id.to_string(),
            protocol: protocol.to_string(),
            api_key: "key".to_string(),
            model: model.to_string(),
            base_url: Some("https://api.example.com/v1".to_string()),
            enabled: true,
            auth_header: None,
        }
    }

    fn route_config(
        default_id: &str,
        connections: Vec<crate::config::ModelConnectionConfig>,
    ) -> crate::config::Config {
        let default_provider = connections
            .iter()
            .find(|connection| connection.id == default_id)
            .map(|connection| connection.provider_id.clone())
            .unwrap_or_else(|| "openai".to_string());
        crate::config::Config {
            llm: crate::config::LLMConfig {
                default_provider,
                default_connection_id: Some(default_id.to_string()),
                connections,
                openai: None,
                anthropic: None,
            },
            ui: crate::config::UiConfig::default(),
            tmdb: crate::config::TmdbConfig::default(),
            execution: crate::tools::execution_isolation::ExecutionIsolationConfig::default(),
            outbound: crate::tools::outbound::OutboundPolicy::default(),
        }
    }

    #[test]
    fn permission_decision_derives_from_audit_and_skips_execution() {
        // P0-4: blocked policy decision → denied, classified by tool + layer.
        let blocked = audit_payload("blocked", "policy_blocks_tool_execution", "run_command");
        let derived = permission_decision_from_audit(&blocked).expect("blocked is a decision");
        assert_eq!(derived.decision, "denied");
        assert_eq!(derived.subject, "command");
        assert_eq!(derived.risk, "destructive");
        assert_eq!(derived.decided_by, "policy");

        // A gate reason maps to the gate layer.
        let gate = audit_payload("blocked", "no_active_task", "write_file");
        let gate_decision = permission_decision_from_audit(&gate).unwrap();
        assert_eq!(gate_decision.decided_by, "gate");
        assert_eq!(gate_decision.subject, "file");

        // Allowed → allowed.
        let allowed = audit_payload("allowed", "policy_allowed", "fetch_web_page");
        let allow_decision = permission_decision_from_audit(&allowed).unwrap();
        assert_eq!(allow_decision.decision, "allowed");
        assert_eq!(allow_decision.subject, "network");

        // Execution outcomes are NOT permission decisions.
        assert!(permission_decision_from_audit(&audit_payload(
            "executed",
            "tool_returned_success",
            "x"
        ))
        .is_none());
        assert!(permission_decision_from_audit(&audit_payload(
            "error",
            "tool_returned_error",
            "x"
        ))
        .is_none());
    }

    #[test]
    fn token_budget_snapshot_uses_run_session_and_day_usage_totals() {
        let db = temp_db();
        db.log_model_usage_event(LogModelUsagePayload {
            session_id: Some("session-a".to_string()),
            run_id: "run-current".to_string(),
            iteration: 1,
            provider: "openai-compatible".to_string(),
            model: "test-model".to_string(),
            input_tokens: 6,
            output_tokens: 4,
            total_tokens: 10,
            source: "model_api_usage".to_string(),
        })
        .unwrap();
        db.log_model_usage_event(LogModelUsagePayload {
            session_id: Some("session-a".to_string()),
            run_id: "run-old".to_string(),
            iteration: 1,
            provider: "openai-compatible".to_string(),
            model: "test-model".to_string(),
            input_tokens: 40,
            output_tokens: 50,
            total_tokens: 90,
            source: "model_api_usage".to_string(),
        })
        .unwrap();
        db.log_model_usage_event(LogModelUsagePayload {
            session_id: Some("session-b".to_string()),
            run_id: "run-other".to_string(),
            iteration: 1,
            provider: "openai-compatible".to_string(),
            model: "test-model".to_string(),
            input_tokens: 11,
            output_tokens: 11,
            total_tokens: 22,
            source: "model_api_usage".to_string(),
        })
        .unwrap();
        db.set_app_state(
            AGENT_TOKEN_BUDGET_POLICY_STATE_KEY,
            serde_json::json!({
                "runSoftLimitTokens": 5,
                "runHardLimitTokens": 20,
                "sessionSoftLimitTokens": 80,
                "sessionHardLimitTokens": 120,
                "daySoftLimitTokens": 100,
                "dayHardLimitTokens": 150,
                "circuitBreakerEnabled": false
            }),
        )
        .unwrap();

        let snapshot = build_agent_token_budget_snapshot(&db, "run-current", Some("session-a"));
        let run = snapshot
            .budgets
            .iter()
            .find(|budget| budget.scope == TokenBudgetScope::Run)
            .unwrap();
        let session = snapshot
            .budgets
            .iter()
            .find(|budget| budget.scope == TokenBudgetScope::Session)
            .unwrap();
        let day = snapshot
            .budgets
            .iter()
            .find(|budget| budget.scope == TokenBudgetScope::Day)
            .unwrap();

        assert_eq!(run.spent_tokens, 10);
        assert_eq!(run.soft_limit_tokens, Some(5));
        assert_eq!(run.hard_limit_tokens, Some(20));
        assert_eq!(session.spent_tokens, 100);
        assert_eq!(day.spent_tokens, 122);
        assert!(!snapshot.circuit_breaker.enabled);
    }

    #[test]
    fn token_budget_policy_can_disable_all_budget_guards() {
        let db = temp_db();
        db.set_app_state(
            AGENT_TOKEN_BUDGET_POLICY_STATE_KEY,
            serde_json::json!(false),
        )
        .unwrap();
        let snapshot = build_agent_token_budget_snapshot(&db, "run-any", Some("session-a"));
        assert!(snapshot.budgets.is_empty());
        assert!(!snapshot.circuit_breaker.enabled);
    }

    #[test]
    fn model_route_request_detects_tools_and_images() {
        let attachment = AgentAttachment {
            id: "image-1".to_string(),
            name: "screen.png".to_string(),
            mime: "image/png".to_string(),
            size: 10,
            kind: "image".to_string(),
            data_url: None,
            text_preview: None,
            island_package_id: None,
        };
        let request = build_model_route_request(
            "请读取这个项目里的文件并验证",
            &[Message::plain(Role::Assistant, "之前的长上下文")],
            "chat",
            &[attachment],
        );
        assert!(request.needs_tools);
        assert!(request.needs_vision);
        assert_eq!(request.attachment_count, 1);
        assert_eq!(request.history_message_count, 1);
        assert!(request.estimated_input_tokens > 0);
    }

    #[test]
    fn model_routing_policy_can_disable_or_force_connection() {
        let db = temp_db();
        db.set_app_state(
            AGENT_MODEL_ROUTING_POLICY_STATE_KEY,
            serde_json::json!(false),
        )
        .unwrap();
        assert!(!agent_model_routing_policy(&db).enabled);

        db.set_app_state(
            AGENT_MODEL_ROUTING_POLICY_STATE_KEY,
            serde_json::json!({
                "forceConnectionId": "strong",
                "preferredTier": "strong"
            }),
        )
        .unwrap();
        let policy = agent_model_routing_policy(&db);
        assert!(policy.enabled);
        assert_eq!(policy.force_connection_id.as_deref(), Some("strong"));
        assert_eq!(
            policy.preferred_tier,
            Some(crate::agent::ModelRouteTier::Strong)
        );
    }

    #[test]
    fn persist_model_route_decision_records_timeline_step() {
        let db = temp_db();
        let session = db.create_session("route").unwrap();
        db.create_agent_run("run-route", Some(&session.id), "default")
            .unwrap();
        let config = route_config(
            "cheap",
            vec![
                route_connection("cheap", "openai", "gpt-4o-mini", "openai-compatible"),
                route_connection("strong", "anthropic", "claude-opus-4-8", "anthropic"),
            ],
        );
        let request =
            ModelRouteRequest::new("chat", "实现 agent 架构任务卡并运行测试", 0, 0, 0, 0, true);
        let decision = select_model_route(&config, &db, &request, &ModelRoutePolicy::default());
        persist_model_route_decision(&db, "run-route", &request, &decision);

        let steps = db.get_agent_run_steps("run-route").unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].step_type, "model_route");
        assert_eq!(steps[0].status, "finished");
        assert_eq!(
            steps[0]
                .output
                .get("selectedConnectionId")
                .and_then(serde_json::Value::as_str),
            Some("strong")
        );
        assert_eq!(
            steps[0]
                .input
                .get("needsTools")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn successful_result_does_not_overwrite_blocked_or_paused_run_status() {
        assert!(should_mark_successful_run_finished(None));
        assert!(should_mark_successful_run_finished(Some("pending")));
        assert!(should_mark_successful_run_finished(Some("running")));
        assert!(!should_mark_successful_run_finished(Some("blocked")));
        assert!(!should_mark_successful_run_finished(Some("paused")));
        assert!(!should_mark_successful_run_finished(Some("failed")));
        assert!(!should_mark_successful_run_finished(Some("cancelled")));
    }

    #[test]
    fn subagent_metadata_is_empty_without_vendored_profiles() {
        let agents = tauri::async_runtime::block_on(get_agent_subagent_metadata()).unwrap();
        assert!(agents.is_empty());
    }

    #[test]
    fn skill_auto_mode_controls_implicit_skill_selection() {
        let db = temp_db();

        assert!(should_auto_apply_skills(&db, "帮我检查这个前端页面"));

        db.set_app_state(AGENT_SKILL_AUTO_MODE_STATE_KEY, serde_json::json!("ask"))
            .unwrap();
        assert!(!should_auto_apply_skills(&db, "帮我检查这个前端页面"));
        assert!(should_auto_apply_skills(
            &db,
            "使用 Skill「frontend」：帮我检查这个前端页面"
        ));

        db.set_app_state(AGENT_SKILL_AUTO_MODE_STATE_KEY, serde_json::json!("off"))
            .unwrap();
        assert!(!should_auto_apply_skills(&db, "帮我检查这个前端页面"));
    }

    #[test]
    fn context_keeps_runtime_context_small_and_recent_messages() {
        let db = temp_db();
        let session = db.create_session("Context").unwrap();
        db.save_profile(serde_json::json!({
            "replyStyle": "minimal",
            "tonePreference": "natural"
        }))
        .unwrap();
        db.add_memory("用户喜欢直接一点的回答", "manual").unwrap();
        for index in 0..26 {
            db.save_message(
                &session.id,
                SaveMessagePayload {
                    id: None,
                    role: if index % 2 == 0 {
                        "user".to_string()
                    } else {
                        "assistant".to_string()
                    },
                    content: format!("old message {index}"),
                    created_at: None,
                    metadata: serde_json::json!({}),
                },
            )
            .unwrap();
        }

        let context = build_conversation_context_from_db(&session.id, &db, "chat").unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!joined.contains("用户画像 JSON"));
        assert!(joined.contains("用户喜欢直接一点的回答"));
        assert!(joined.contains("old message 25"));
        assert!(context
            .iter()
            .any(|message| matches!(message.role, Role::System)
                && message.content.contains("压缩摘要")));
    }

    #[test]
    fn session_context_window_selection_is_observable() {
        let db = temp_db();
        let session = db.create_session("Context audit").unwrap();
        for index in 0..13 {
            db.save_message(
                &session.id,
                SaveMessagePayload {
                    id: None,
                    role: if index % 2 == 0 {
                        "user".to_string()
                    } else {
                        "assistant".to_string()
                    },
                    content: format!("durable message {index}"),
                    created_at: None,
                    metadata: serde_json::json!({}),
                },
            )
            .unwrap();
        }

        let window = build_session_context_window_from_db_with_history_mode(
            &session.id,
            &db,
            "chat",
            ConversationHistoryMode::Full,
        )
        .unwrap();
        let joined = window
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(joined.contains("[ContextWindow 选择策略]"));
        assert!(joined.contains("source=persistent_session"));
        assert!(joined.contains("durableUserAssistantMessages=13"));
        assert!(joined.contains("selectedRecentMessages=12"));
        assert!(joined.contains("omittedOlderMessages=1"));
        assert!(joined.contains("Session/EventLog 是持久事实源"));
        assert!(!joined.contains("durable message 0"));
        assert!(joined.contains("durable message 12"));
    }

    #[test]
    fn session_context_window_pins_persisted_plan_and_active_task() {
        let db = temp_db();
        let session = db.create_session("Context task anchor").unwrap();
        db.create_agent_run("run-existing", Some(&session.id), "default")
            .unwrap();
        let plan = db
            .create_run_plan(
                &session.id,
                "实现 Session 与上下文窗口分层",
                Some("run-existing"),
                Some(&serde_json::json!(["窗口选择策略可观察"])),
                Some("模型输入中始终能看到当前任务锚点"),
                Some(&serde_json::json!(["不做 UI"])),
            )
            .unwrap();
        let task = db
            .create_plan_task_full(
                &session.id,
                "拆分持久 Session 和临时 ContextWindow",
                None,
                Some("run-existing"),
                "test",
                None,
                None,
            )
            .unwrap();
        db.set_active_plan_task(&session.id, Some(&task.id))
            .unwrap();

        let context = build_conversation_context_from_db(&session.id, &db, "chat").unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(joined.contains("[ContextWindow 当前持久任务锚点]"));
        assert!(joined.contains(&format!("planId={}", plan.id)));
        assert!(joined.contains("goal=实现 Session 与上下文窗口分层"));
        assert!(joined.contains("observableOutcome=模型输入中始终能看到当前任务锚点"));
        assert!(joined.contains(&format!("activeTaskId={}", task.id)));
        assert!(joined.contains("activeTaskTitle=拆分持久 Session 和临时 ContextWindow"));
        assert!(joined.contains("activeTaskStatus=pending"));
        assert!(joined.contains("activeTaskEvidenceStatus=none"));
        assert!(joined.contains("planAnchorPinned=true"));
        assert!(joined.contains("activeTaskPinned=true"));
    }

    #[test]
    fn context_compression_is_observable_and_keeps_current_task_pin() {
        let db = temp_db();
        let session = db.create_session("Compression audit").unwrap();
        db.create_agent_run("run-compress", Some(&session.id), "default")
            .unwrap();
        let plan = db
            .create_run_plan(
                &session.id,
                "长会话压缩时仍保留当前任务目标全句",
                Some("run-compress"),
                None,
                Some("模型输入不会因为旧消息摘要而丢失当前任务目标"),
                None,
            )
            .unwrap();
        let task = db
            .create_plan_task_full(
                &session.id,
                "实现可观测上下文压缩策略",
                None,
                Some("run-compress"),
                "test",
                None,
                None,
            )
            .unwrap();
        db.set_active_plan_task(&session.id, Some(&task.id))
            .unwrap();
        for index in 0..30 {
            db.save_message(
                &session.id,
                SaveMessagePayload {
                    id: None,
                    role: if index % 2 == 0 {
                        "user".to_string()
                    } else {
                        "assistant".to_string()
                    },
                    content: format!("compressible durable message {index}"),
                    created_at: None,
                    metadata: serde_json::json!({}),
                },
            )
            .unwrap();
        }

        let context = build_conversation_context_from_db(&session.id, &db, "chat").unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(joined.contains("compressionTriggered=true"));
        assert!(joined.contains("compressionReason=message_count>24+summary_included"));
        assert!(joined.contains("summaryIncluded=true"));
        assert!(joined.contains("selectedRecentMessages=12"));
        assert!(joined.contains("omittedOlderMessages=18"));
        assert!(joined.contains("[ContextWindow 当前持久任务锚点]"));
        assert!(joined.contains(&format!("planId={}", plan.id)));
        assert!(joined.contains("goal=长会话压缩时仍保留当前任务目标全句"));
        assert!(joined.contains("observableOutcome=模型输入不会因为旧消息摘要而丢失当前任务目标"));
        assert!(joined.contains(&format!("activeTaskId={}", task.id)));
        assert!(joined.contains("activeTaskTitle=实现可观测上下文压缩策略"));
        assert!(joined.contains("compressible durable message 29"));
    }

    #[test]
    fn context_compression_budget_prunes_recent_tail_but_preserves_task_anchor() {
        let db = temp_db();
        let session = db.create_session("Budget compression").unwrap();
        db.create_agent_run("run-budget", Some(&session.id), "default")
            .unwrap();
        let plan = db
            .create_run_plan(
                &session.id,
                "预算压缩不能丢当前目标和任务锚点",
                Some("run-budget"),
                None,
                Some("旧消息很长时仍然保留当前目标和任务锚点"),
                None,
            )
            .unwrap();
        let task = db
            .create_plan_task_full(
                &session.id,
                "压缩预算保护当前任务",
                None,
                Some("run-budget"),
                "test",
                None,
                None,
            )
            .unwrap();
        db.set_active_plan_task(&session.id, Some(&task.id))
            .unwrap();
        for index in 0..12 {
            db.save_message(
                &session.id,
                SaveMessagePayload {
                    id: None,
                    role: if index % 2 == 0 {
                        "user".to_string()
                    } else {
                        "assistant".to_string()
                    },
                    content: format!("budget-heavy-message-{index}-{}", "x".repeat(7_000)),
                    created_at: None,
                    metadata: serde_json::json!({}),
                },
            )
            .unwrap();
        }

        let context = build_conversation_context_from_db(&session.id, &db, "chat").unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let selected_history = context
            .iter()
            .filter(|message| matches!(message.role, Role::User | Role::Assistant))
            .filter(|message| message.content.starts_with("budget-heavy-message-"))
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            selected_history.len(),
            CONTEXT_WINDOW_PROTECTED_RECENT_MESSAGE_FLOOR
        );
        assert!(selected_history[0].starts_with("budget-heavy-message-8-"));
        assert!(selected_history[3].starts_with("budget-heavy-message-11-"));
        assert!(joined.contains("compressionTriggered=true"));
        assert!(joined.contains("soft_char_budget>24000"));
        assert!(joined.contains("recent_tail_pruned_to_budget"));
        assert!(joined.contains("protected_recent_floor_preserved"));
        assert!(joined.contains("summaryIncluded=true"));
        assert!(joined.contains("selectedRecentMessages=4"));
        assert!(joined.contains("omittedOlderMessages=8"));
        assert!(joined.contains(&format!("planId={}", plan.id)));
        assert!(joined.contains("goal=预算压缩不能丢当前目标和任务锚点"));
        assert!(joined.contains("activeTaskTitle=压缩预算保护当前任务"));
        assert!(joined.contains(&format!("activeTaskId={}", task.id)));
    }

    #[test]
    fn context_injects_enabled_long_term_memories() {
        let db = temp_db();
        let session = db.create_session("Memory filters").unwrap();
        let disabled = db.add_memory("不要注入这条停用记忆", "manual").unwrap();
        db.update_memory(&disabled.id, None, Some(false)).unwrap();
        db.add_memory("保留这条启用记忆", "manual").unwrap();

        let context = build_conversation_context_from_db(&session.id, &db, "chat").unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(joined.contains("启用的本地长期记忆"));
        assert!(joined.contains("保留这条启用记忆"));
        assert!(!joined.contains("不要注入这条停用记忆"));
    }

    #[test]
    fn context_respects_memory_injection_setting() {
        let db = temp_db();
        let session = db.create_session("Memory disabled").unwrap();
        db.add_memory("不要注入这条记忆", "manual").unwrap();
        db.set_app_state(MEMORY_INJECTION_STATE_KEY, serde_json::json!(false))
            .unwrap();

        let context = build_conversation_context_from_db(&session.id, &db, "chat").unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!joined.contains("启用的本地长期记忆"));
        assert!(!joined.contains("不要注入这条记忆"));
    }

    #[test]
    fn plan_mode_context_disables_execution_boundary() {
        let db = temp_db();
        let session = db.create_session("Plan mode").unwrap();

        let context = build_conversation_context_from_db(&session.id, &db, "plan").unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(joined.contains("当前是计划模式"));
        assert!(joined.contains("工具策略：只允许读取和整理方案"));
        assert!(joined.contains("只能读取必要信息"));
        assert!(joined.contains("不要写文件"));
    }

    #[test]
    fn code_review_mode_is_read_only_without_plan_output_boundary() {
        let db = temp_db();
        let session = db.create_session("Code review mode").unwrap();

        let context = build_conversation_context_from_db(&session.id, &db, "code_review").unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(joined.contains("当前是代码审查模式"));
        assert!(joined.contains("工具策略：只允许读取和整理方案"));
        assert!(joined.contains("findings first"));
        assert!(joined.contains("不要写文件"));
        assert!(joined.contains("不要运行命令"));
        assert!(!joined.contains("等待用户确认的计划"));
    }

    #[test]
    fn code_review_subagent_instruction_hides_executable_profile_steps() {
        let profile = AgentProfile {
            metadata: AgentProfileMetadata {
                name: "code-reviewer".to_string(),
                description: "Expert code review specialist.".to_string(),
                model: "sonnet".to_string(),
                tools: vec![
                    "Read".to_string(),
                    "Grep".to_string(),
                    "Glob".to_string(),
                    "Bash".to_string(),
                ],
                source: "builtin/agents/code-reviewer.md".to_string(),
                source_kind: "builtin".to_string(),
            },
            instructions: "Run git diff and tests. MUST BE USED.".to_string(),
        };

        let instruction = build_subagent_instruction(&profile, "code_review");

        assert!(instruction.contains("代码审查命令硬性边界"));
        assert!(instruction.contains("Allowed review capabilities: Read, Grep, Glob"));
        assert!(!instruction.contains("Declared tools"));
        assert!(!instruction.contains("Bash"));
        assert!(!instruction.contains("Run git diff"));
        assert!(!instruction.contains("MUST BE USED"));
    }

    #[test]
    fn plan_permission_context_outputs_plan_boundary() {
        let db = temp_db();
        let session = db.create_session("Permission plan").unwrap();
        db.set_app_state("agent_permission_mode", serde_json::json!("plan"))
            .unwrap();

        let context = build_conversation_context_from_db(&session.id, &db, "chat").unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(joined.contains("当前权限档位是计划模式"));
        assert!(joined.contains("不写文件"));
        assert!(joined.contains("不运行命令"));
        assert!(joined.contains("默认模式或完全访问模式"));
    }

    #[test]
    fn tool_policy_maps_mode_and_permission_level() {
        assert_eq!(
            tool_access_policy_for("plan", "full_access"),
            ToolAccessPolicy::Plan
        );
        assert_eq!(
            tool_access_policy_for("code_review", "full_access"),
            ToolAccessPolicy::Plan
        );
        assert_eq!(
            tool_access_policy_for("chat", "plan"),
            ToolAccessPolicy::Plan
        );
        assert_eq!(
            tool_access_policy_for("chat", "default"),
            ToolAccessPolicy::Default
        );
        assert_eq!(
            tool_access_policy_for("chat", "full_access"),
            ToolAccessPolicy::FullAccess
        );
        assert_eq!(
            tool_access_policy_for("chat", "unexpected"),
            ToolAccessPolicy::Default
        );
    }

    #[test]
    fn context_includes_permission_policy_mapping() {
        let db = temp_db();
        let session = db.create_session("Permission policy").unwrap();
        db.set_app_state("agent_permission_mode", serde_json::json!("default"))
            .unwrap();

        let context = build_conversation_context_from_db(&session.id, &db, "chat").unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(joined.contains("Agent 权限模式：default（默认模式）"));
        assert!(joined.contains("工具策略：文件修改可执行，命令需要确认"));
        assert!(joined.contains("默认模式=普通文件修改直接执行，运行命令需要确认卡片"));
    }

    #[test]
    fn context_is_built_before_current_v2_message_is_saved() {
        let db = temp_db();
        let session = db.create_session("No duplicate current").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "previous question".to_string(),
                created_at: None,
                metadata: serde_json::json!({}),
            },
        )
        .unwrap();

        let context = build_conversation_context_from_db(&session.id, &db, "chat").unwrap();
        assert!(context
            .iter()
            .any(|message| message.content == "previous question"));
        assert!(!context
            .iter()
            .any(|message| message.content == "current question"));
    }

    #[test]
    fn standalone_question_context_drops_old_task_history() {
        let db = temp_db();
        let session = db.create_session("Standalone question").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "帮我做一个灵动岛网页，放在桌面。".to_string(),
                created_at: None,
                metadata: serde_json::json!({}),
            },
        )
        .unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "assistant".to_string(),
                content: "现在开始搭建灵动岛。先创建项目目录和所有源文件。".to_string(),
                created_at: None,
                metadata: serde_json::json!({}),
            },
        )
        .unwrap();

        let mode =
            if user_message_should_use_conversation_history("deepseek 的模型接口不接受图片吗")
            {
                ConversationHistoryMode::Full
            } else {
                ConversationHistoryMode::CurrentMessageOnly
            };
        let context =
            build_conversation_context_from_db_with_history_mode(&session.id, &db, "chat", mode)
                .unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(joined.contains("新的独立当前轮请求"));
        assert!(!joined.contains("灵动岛"));
        assert!(!joined.contains("创建项目目录"));
    }

    #[test]
    fn unrelated_new_task_context_drops_old_task_history() {
        let db = temp_db();
        let session = db.create_session("Unrelated new task").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "帮我做一个灵动岛网页，放在桌面。".to_string(),
                created_at: None,
                metadata: serde_json::json!({}),
            },
        )
        .unwrap();

        let mode = if user_message_should_use_conversation_history("新建一个 todo.html") {
            ConversationHistoryMode::Full
        } else {
            ConversationHistoryMode::CurrentMessageOnly
        };
        let context =
            build_conversation_context_from_db_with_history_mode(&session.id, &db, "chat", mode)
                .unwrap();
        let joined = context
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!joined.contains("灵动岛"));
        assert!(joined.contains("新的独立当前轮请求"));
    }

    #[test]
    fn explicit_continuation_context_keeps_old_task_history() {
        let db = temp_db();
        let session = db.create_session("Continuation").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "帮我做一个灵动岛网页，放在桌面。".to_string(),
                created_at: None,
                metadata: serde_json::json!({}),
            },
        )
        .unwrap();

        let mode = if user_message_should_use_conversation_history("继续按刚才计划执行") {
            ConversationHistoryMode::Full
        } else {
            ConversationHistoryMode::CurrentMessageOnly
        };
        let context =
            build_conversation_context_from_db_with_history_mode(&session.id, &db, "chat", mode)
                .unwrap();

        assert!(context
            .iter()
            .any(|message| message.content.contains("灵动岛")));
    }

    #[test]
    fn historical_image_attachments_are_not_replayed_to_provider() {
        let db = temp_db();
        let session = db.create_session("Image replay").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "看这张图".to_string(),
                created_at: None,
                metadata: serde_json::json!({
                    "attachments": [{
                        "id": "img-1",
                        "name": "screen.png",
                        "mime": "image/png",
                        "size": 12,
                        "kind": "image",
                        "dataUrl": "data:image/png;base64,aaaa"
                    }]
                }),
            },
        )
        .unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "deepseek 的模型接口不接受图片吗".to_string(),
                created_at: None,
                metadata: serde_json::json!({}),
            },
        )
        .unwrap();

        let context = build_conversation_context_from_db(&session.id, &db, "chat").unwrap();
        let history_image_message = context
            .iter()
            .find(|message| message.content == "看这张图")
            .expect("historical image message should remain in text history");

        assert!(history_image_message.attachments.is_empty());
        assert!(context
            .iter()
            .any(|message| message.content == "deepseek 的模型接口不接受图片吗"));
    }

    #[test]
    fn pending_command_tool_result_is_saved_as_recoverable_tool_message() {
        let db = temp_db();
        let session = db.create_session("Pending command").unwrap();
        let run = db
            .create_agent_run("run-pending-command", Some(&session.id), "default")
            .unwrap();
        let pending_id = "pcmd_test_restore";
        let payload = serde_json::json!({
            "status": "warning",
            "summary": "命令预览已准备，尚未运行。",
            "data": {
                "pendingCommand": {
                    "id": pending_id,
                    "command": "Write-Output aura",
                    "cwd": ".",
                    "reason": "测试命令",
                    "shell": "powershell"
                }
            },
            "next_actions": [],
            "recoverable": true
        });

        persist_agent_event(
            &db,
            &run.id,
            Some(&session.id),
            &AgentEvent::ToolResult {
                result: payload.to_string(),
            },
        );

        let messages = db.get_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, pending_command_message_id(pending_id));
        assert_eq!(messages[0].role, "tool");
        assert!(messages[0].content.contains("pendingCommand"));
        assert_eq!(
            db.find_run_id_for_pending_command(pending_id)
                .unwrap()
                .as_deref(),
            Some(run.id.as_str())
        );
    }

    #[test]
    fn pending_command_resolution_replaces_recoverable_tool_message() {
        let db = temp_db();
        let session = db.create_session("Pending command resolution").unwrap();
        let run = db
            .create_agent_run(
                "run-pending-command-resolution",
                Some(&session.id),
                "default",
            )
            .unwrap();
        let pending_id = "pcmd_test_resolution";
        let pending_payload = serde_json::json!({
            "status": "warning",
            "summary": "命令预览已准备，尚未运行。",
            "data": {
                "pendingCommand": {
                    "id": pending_id,
                    "command": "Write-Output aura",
                    "cwd": ".",
                    "reason": "测试命令",
                    "shell": "powershell"
                }
            },
            "next_actions": [],
            "recoverable": true
        });

        persist_agent_event(
            &db,
            &run.id,
            Some(&session.id),
            &AgentEvent::ToolResult {
                result: pending_payload.to_string(),
            },
        );
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: Some(pending_command_message_id(pending_id)),
                role: "tool".to_string(),
                content: serde_json::json!({
                    "status": "success",
                    "summary": "用户确认后命令运行结束，退出码：0。",
                    "data": {
                        "pendingCommandId": pending_id,
                        "confirmed": true,
                        "commandResult": {
                            "command": "Write-Output aura",
                            "cwd": ".",
                            "exitCode": 0,
                            "stdout": "aura",
                            "stderr": "",
                            "timedOut": false
                        }
                    },
                    "next_actions": [],
                    "recoverable": false
                })
                .to_string(),
                created_at: None,
                metadata: serde_json::json!({ "source": "agent_pending_command_resolution" }),
            },
        )
        .unwrap();

        let messages = db.get_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, pending_command_message_id(pending_id));
        assert!(!messages[0].content.contains("\"pendingCommand\""));
        assert!(messages[0].content.contains("\"commandResult\""));
    }
}
