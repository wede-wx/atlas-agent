use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tauri::{Emitter, State, Window};
use tokio::sync::mpsc;

use crate::agent::{AgentEvent, ToolResult};
use crate::browser_automation::{list_browser_audit_events, BrowserAutomationAuditEvent};
use crate::commands::agent::pending_command_message_id;
use crate::storage::{
    ActivityEvent, ArtifactRecord, ConversationHistoryReport, FileWritePreview, LocalDbHealth,
    LogActivityEventPayload, MemoryRecord, MessageRecord, PersonalityProgressRecord,
    PlanChangeRecord, PlanTaskPatch, PlanTaskRecord, PlanTaskRunIntegrityReport,
    PluginCapabilityEventRecord, PluginPackageRecord, ProfileRecord, ProjectRecord,
    ProviderCapabilitiesRow, RecordArtifactPayload, ResetLocalDataOptions, ResetLocalDataSummary,
    SaveMessagePayload, SessionRecord,
};
use crate::tools::Tool;
use crate::{tools, AppState};

#[tauri::command]
pub async fn init_local_db(state: State<'_, AppState>) -> Result<String, String> {
    state.local_db.init().map_err(|e| e.to_string())?;
    Ok(state.local_db.path().to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_sessions(state: State<'_, AppState>) -> Result<Vec<SessionRecord>, String> {
    state.local_db.list_sessions().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<ProjectRecord>, String> {
    state.local_db.list_projects().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn upsert_project(
    title: Option<String>,
    root_path: String,
    state: State<'_, AppState>,
) -> Result<ProjectRecord, String> {
    let root = canonical_project_folder(&root_path)?;
    state
        .local_db
        .upsert_project(
            title.as_deref().unwrap_or(""),
            Some(&root.to_string_lossy()),
            "folder",
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_project_folder(
    title: String,
    state: State<'_, AppState>,
) -> Result<ProjectRecord, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("请输入项目名称。".to_string());
    }
    let parent = default_project_parent()?;
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let folder_name = safe_project_folder_name(title);
    let mut path = parent.join(&folder_name);
    let mut index = 2;
    while path.exists() {
        path = parent.join(format!("{folder_name}-{index}"));
        index += 1;
    }
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    state
        .local_db
        .upsert_project(title, Some(&path.to_string_lossy()), "folder")
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_project(
    id: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<ProjectRecord, String> {
    state
        .local_db
        .rename_project(&id, &title)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_project_pinned(
    id: String,
    pinned: bool,
    state: State<'_, AppState>,
) -> Result<ProjectRecord, String> {
    state
        .local_db
        .set_project_pinned(&id, pinned)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn archive_project(
    id: String,
    state: State<'_, AppState>,
) -> Result<ProjectRecord, String> {
    state
        .local_db
        .archive_project(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_project(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ProjectRecord>, String> {
    state
        .local_db
        .delete_project(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_project_in_explorer(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let project = state.local_db.get_project(&id).map_err(|e| e.to_string())?;
    let root = project
        .root_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "这个项目没有关联文件夹。".to_string())?;
    let path = Path::new(root);
    if !path.exists() || !path.is_dir() {
        return Err("项目文件夹不存在。".to_string());
    }
    open_folder_in_file_manager(path)
}

#[tauri::command]
pub async fn generate_history_report(
    range: Option<String>,
    state: State<'_, AppState>,
) -> Result<ConversationHistoryReport, String> {
    state
        .local_db
        .conversation_history_report(range.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_archived_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<SessionRecord>, String> {
    state
        .local_db
        .list_archived_sessions()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn search_sessions(
    query: String,
    archived: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<SessionRecord>, String> {
    if archived.unwrap_or(false) {
        return state
            .local_db
            .search_archived_sessions(&query)
            .map_err(|e| e.to_string());
    }
    state
        .local_db
        .search_sessions(&query)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_session(
    title: String,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<SessionRecord, String> {
    state
        .local_db
        .create_session_for_project(&title, project_id.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_session(
    id: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<SessionRecord, String> {
    state
        .local_db
        .rename_session(&id, &title)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_session(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SessionRecord>, String> {
    state
        .local_db
        .delete_session(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn archive_session(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SessionRecord>, String> {
    state
        .local_db
        .archive_session(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restore_session(
    id: String,
    state: State<'_, AppState>,
) -> Result<SessionRecord, String> {
    state
        .local_db
        .restore_session(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_session_pinned(
    id: String,
    pinned: bool,
    state: State<'_, AppState>,
) -> Result<SessionRecord, String> {
    state
        .local_db
        .set_session_pinned(&id, pinned)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_messages(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<MessageRecord>, String> {
    state
        .local_db
        .get_messages(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_message(
    session_id: String,
    message: SaveMessagePayload,
    state: State<'_, AppState>,
) -> Result<MessageRecord, String> {
    state
        .local_db
        .save_message(&session_id, message)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_session_context(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    state
        .local_db
        .clear_session_context(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_plan_tasks(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<PlanTaskRecord>, String> {
    state
        .local_db
        .list_plan_tasks(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_plan_task(
    session_id: String,
    title: String,
    parent_id: Option<String>,
    run_id: Option<String>,
    source: Option<String>,
    change_reason: Option<String>,
    state: State<'_, AppState>,
) -> Result<PlanTaskRecord, String> {
    state
        .local_db
        .create_plan_task_full_with_reason(
            &session_id,
            &title,
            parent_id.as_deref(),
            run_id.as_deref(),
            source.as_deref().unwrap_or("manual"),
            None,
            None,
            change_reason
                .as_deref()
                .unwrap_or("用户通过本地命令创建计划任务。"),
            "user",
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_plan_task(
    payload: PlanTaskPatch,
    change_reason: Option<String>,
    state: State<'_, AppState>,
) -> Result<PlanTaskRecord, String> {
    state
        .local_db
        .update_plan_task_with_reason(
            payload,
            change_reason
                .as_deref()
                .unwrap_or("用户通过本地命令编辑计划任务。"),
            "user",
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_plan_task_status(
    id: String,
    status: String,
    run_id: Option<String>,
    change_reason: Option<String>,
    state: State<'_, AppState>,
) -> Result<PlanTaskRecord, String> {
    state
        .local_db
        .update_plan_task_status_with_reason(
            &id,
            &status,
            run_id.as_deref(),
            change_reason
                .as_deref()
                .unwrap_or("用户通过本地命令更新计划任务状态。"),
            "user",
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn archive_plan_task(
    id: String,
    change_reason: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .local_db
        .archive_plan_task_with_reason(
            &id,
            change_reason
                .as_deref()
                .unwrap_or("用户通过本地命令归档计划任务。"),
            "user",
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_active_plan_task(
    session_id: String,
    task_id: Option<String>,
    change_reason: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .local_db
        .set_active_plan_task_with_reason(
            &session_id,
            task_id.as_deref(),
            change_reason
                .as_deref()
                .unwrap_or("用户通过本地命令切换活跃任务。"),
            "user",
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn waive_plan_task(
    id: String,
    reason: Option<String>,
    state: State<'_, AppState>,
) -> Result<PlanTaskRecord, String> {
    // Status → waived + evidence_status → waived. Records a brief evidence note
    // so final_audit can attribute the waiver.
    state
        .local_db
        .update_plan_task_status_with_reason(
            &id,
            "waived",
            None,
            reason.as_deref().unwrap_or("用户通过 UI 标记任务豁免。"),
            "user",
        )
        .map_err(|e| e.to_string())?;
    let reason_text = reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("用户在 UI 中标记为豁免。");
    state
        .local_db
        .update_plan_task_evidence_with_reason(
            &id,
            Some(&serde_json::json!({
                "waived_by": "user_ui",
                "reason": reason_text,
            })),
            "waived",
            None,
            reason_text,
            "user",
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_plan_change_events(
    session_id: String,
    run_id: Option<String>,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<PlanChangeRecord>, String> {
    state
        .local_db
        .list_plan_change_events(&session_id, run_id.as_deref(), limit.unwrap_or(100))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn scan_plan_task_run_integrity(
    state: State<'_, AppState>,
) -> Result<PlanTaskRunIntegrityReport, String> {
    state
        .local_db
        .scan_plan_task_run_integrity()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn repair_plan_task_run_integrity(
    change_reason: Option<String>,
    state: State<'_, AppState>,
) -> Result<PlanTaskRunIntegrityReport, String> {
    state
        .local_db
        .repair_plan_task_run_integrity(
            change_reason
                .as_deref()
                .unwrap_or("清理无效 plan task run_id 引用。"),
            "system",
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_plugin_packages(
    state: State<'_, AppState>,
) -> Result<Vec<PluginPackageRecord>, String> {
    state
        .local_db
        .list_plugin_packages()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_plugin_package_enabled(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<PluginPackageRecord, String> {
    state
        .local_db
        .set_plugin_package_enabled(&id, enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_plugin_capability_events(
    plugin_id: Option<String>,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<PluginCapabilityEventRecord>, String> {
    state
        .local_db
        .list_plugin_capability_events(plugin_id.as_deref(), limit.unwrap_or(100))
        .map_err(|e| e.to_string())
}

async fn execute_tool<T: Tool>(tool: T, args: Value) -> Result<ToolResult, String> {
    tool.execute(args).await.map_err(|error| error.to_string())
}

fn git_write_roots(cwd: &Option<String>) -> Vec<PathBuf> {
    cwd.as_ref().map(PathBuf::from).into_iter().collect()
}

#[tauri::command]
pub async fn git_status(cwd: Option<String>) -> Result<ToolResult, String> {
    execute_tool(tools::GitStatusTool, json!({ "cwd": cwd })).await
}

#[tauri::command]
pub async fn git_diff(
    cwd: Option<String>,
    staged: Option<bool>,
    stat: Option<bool>,
    refs: Option<Vec<String>>,
    paths: Option<Vec<String>>,
) -> Result<ToolResult, String> {
    execute_tool(
        tools::GitDiffTool,
        json!({
            "cwd": cwd,
            "staged": staged.unwrap_or(false),
            "stat": stat.unwrap_or(false),
            "refs": refs.unwrap_or_default(),
            "paths": paths.unwrap_or_default(),
        }),
    )
    .await
}

#[tauri::command]
pub async fn git_log(
    cwd: Option<String>,
    limit: Option<u64>,
    reference: Option<String>,
    paths: Option<Vec<String>>,
) -> Result<ToolResult, String> {
    execute_tool(
        tools::GitLogTool,
        json!({
            "cwd": cwd,
            "limit": limit.unwrap_or(20),
            "ref": reference,
            "paths": paths.unwrap_or_default(),
        }),
    )
    .await
}

#[tauri::command]
pub async fn git_show(
    cwd: Option<String>,
    reference: String,
    stat: Option<bool>,
) -> Result<ToolResult, String> {
    execute_tool(
        tools::GitShowTool,
        json!({
            "cwd": cwd,
            "ref": reference,
            "stat": stat.unwrap_or(false),
        }),
    )
    .await
}

#[tauri::command]
pub async fn git_stage(
    cwd: Option<String>,
    paths: Option<Vec<String>>,
    all: Option<bool>,
    confirmed: Option<bool>,
) -> Result<ToolResult, String> {
    let tool = tools::GitStageTool::new_with_roots(git_write_roots(&cwd));
    execute_tool(
        tool,
        json!({
            "cwd": cwd,
            "paths": paths.unwrap_or_default(),
            "all": all.unwrap_or(false),
            "confirmed": confirmed.unwrap_or(false),
        }),
    )
    .await
}

#[tauri::command]
pub async fn git_commit(
    cwd: Option<String>,
    message: String,
    confirmed: Option<bool>,
) -> Result<ToolResult, String> {
    let tool = tools::GitCommitTool::new_with_roots(git_write_roots(&cwd));
    execute_tool(
        tool,
        json!({
            "cwd": cwd,
            "message": message,
            "confirmed": confirmed.unwrap_or(false),
        }),
    )
    .await
}

#[tauri::command]
pub async fn git_create_branch(
    cwd: Option<String>,
    branch: String,
    start_point: Option<String>,
    confirmed: Option<bool>,
) -> Result<ToolResult, String> {
    let tool = tools::GitCreateBranchTool::new_with_roots(git_write_roots(&cwd));
    execute_tool(
        tool,
        json!({
            "cwd": cwd,
            "branch": branch,
            "startPoint": start_point,
            "confirmed": confirmed.unwrap_or(false),
        }),
    )
    .await
}

#[tauri::command]
pub async fn git_push(
    cwd: Option<String>,
    remote: String,
    branch: String,
    set_upstream: Option<bool>,
    confirmed: Option<bool>,
) -> Result<ToolResult, String> {
    let tool = tools::GitPushTool::new_with_roots(git_write_roots(&cwd));
    execute_tool(
        tool,
        json!({
            "cwd": cwd,
            "remote": remote,
            "branch": branch,
            "setUpstream": set_upstream.unwrap_or(false),
            "confirmed": confirmed.unwrap_or(false),
        }),
    )
    .await
}

#[tauri::command]
pub async fn install_plugin_package(
    manifest: Value,
    enabled: Option<bool>,
    trusted: Option<bool>,
    confirmed: Option<bool>,
    state: State<'_, AppState>,
) -> Result<ToolResult, String> {
    execute_tool(
        tools::InstallPluginPackageTool::new(state.local_db.clone()),
        json!({
            "manifest": manifest,
            "enabled": enabled.unwrap_or(false),
            "trusted": trusted.unwrap_or(false),
            "confirmed": confirmed.unwrap_or(false),
        }),
    )
    .await
}

#[tauri::command]
pub async fn invoke_plugin_capability(
    plugin_id: String,
    capability_id: String,
    input: Option<Value>,
    confirmed: Option<bool>,
    state: State<'_, AppState>,
) -> Result<ToolResult, String> {
    execute_tool(
        tools::InvokePluginCapabilityTool::new(state.local_db.clone()),
        json!({
            "pluginId": plugin_id,
            "capabilityId": capability_id,
            "input": input.unwrap_or_else(|| json!({})),
            "confirmed": confirmed.unwrap_or(false),
        }),
    )
    .await
}

/// M5: read-only listing of the provider_capabilities table for the settings UI.
#[tauri::command]
pub async fn list_provider_capabilities(
    state: State<'_, AppState>,
) -> Result<Vec<ProviderCapabilitiesRow>, String> {
    state
        .local_db
        .list_provider_capabilities()
        .map_err(|e| e.to_string())
}

/// M5: resolve (and persist on first use) the cap row for a given pair so the UI
/// can display vision/tool_calls badges next to the active connection.
#[tauri::command]
pub async fn resolve_provider_capabilities(
    provider_id: String,
    model: String,
    state: State<'_, AppState>,
) -> Result<ProviderCapabilitiesRow, String> {
    let caps =
        crate::agent::capabilities::resolve_capabilities(&state.local_db, &provider_id, &model)
            .map_err(|e| e.to_string())?;
    Ok(caps.into_row())
}

/// T33: drop all capability rows for a provider so the next resolve falls back
/// to the builtin defaults and writes a fresh capability_audit row.
#[tauri::command]
pub async fn reset_provider_capabilities(
    provider_id: String,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    state
        .local_db
        .reset_capabilities_for_provider(&provider_id)
        .map_err(|e| e.to_string())
}

/// T30: capability audit log (read-only).
#[tauri::command]
pub async fn list_capability_audit(
    provider_id: Option<String>,
    model: Option<String>,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<crate::storage::CapabilityAuditRow>, String> {
    state
        .local_db
        .list_capability_audit(
            provider_id.as_deref(),
            model.as_deref(),
            limit.unwrap_or(100),
        )
        .map_err(|e| e.to_string())
}

/// T28: user override — write a capability row with source=user_override.
#[tauri::command]
pub async fn override_provider_capabilities(
    provider_id: String,
    model: String,
    vision: bool,
    tool_calls: bool,
    json_mode: bool,
    max_context: u32,
    state: State<'_, AppState>,
) -> Result<ProviderCapabilitiesRow, String> {
    let row = ProviderCapabilitiesRow {
        provider_id: provider_id.clone(),
        model: model.clone(),
        vision,
        tool_calls,
        json_mode,
        max_context,
        source: "user_override".to_string(),
        updated_at: 0,
    };
    state
        .local_db
        .upsert_provider_capabilities(&row)
        .map_err(|e| e.to_string())?;
    state
        .local_db
        .get_provider_capabilities(&provider_id, &model)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "row missing after upsert".to_string())
}

/// T22: live HTTP probe of the provider endpoint. Looks up connection details
/// from `config.toml`, performs a minimal completion dry-run where supported,
/// and separately hits `/v1/models` or `/api/tags`. A conclusive dry-run
/// upgrades the cap row's `source` to `verified`; endpoint-only success can
/// only upgrade to `probed`.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeProviderResult {
    pub probe: crate::agent::probe::EndpointProbe,
    pub dry_run: crate::agent::probe::CapabilityDryRunReport,
    pub capabilities: ProviderCapabilitiesRow,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn probe_provider_capabilities(
    provider_id: String,
    model: String,
    state: State<'_, AppState>,
) -> Result<ProbeProviderResult, String> {
    let config = crate::config::Config::load().map_err(|e| e.to_string())?;
    let connection = config
        .llm
        .connections
        .iter()
        .find(|c| c.provider_id == provider_id && c.model == model && c.enabled)
        .or_else(|| {
            config
                .llm
                .connections
                .iter()
                .find(|c| c.provider_id == provider_id && c.enabled)
        })
        .ok_or_else(|| format!("no enabled connection for provider '{provider_id}'"))?;

    let input = crate::agent::probe::ProbeInput {
        protocol: connection.protocol.clone(),
        base_url: connection
            .base_url
            .clone()
            .unwrap_or_else(|| default_base_url_for(&connection.provider_id, &connection.protocol)),
        api_key: connection.api_key.clone(),
        model: model.clone(),
        auth_header: connection.auth_header.clone(),
    };

    // Always resolve current cap row (persists builtin if missing).
    let mut caps =
        crate::agent::capabilities::resolve_capabilities(&state.local_db, &provider_id, &model)
            .map_err(|e| e.to_string())?;
    let dry_run = crate::agent::probe::probe_capability_dry_run(&input, &caps).await;
    let probe_result = crate::agent::probe::probe_endpoint(&input).await;

    match probe_result {
        Ok(probe) => {
            let changed = apply_probe_results_to_capabilities(&mut caps, &probe, &dry_run);
            if changed {
                let row = caps.clone().into_row();
                let _ = state.local_db.upsert_provider_capabilities(&row);
            }
            Ok(ProbeProviderResult {
                probe,
                dry_run,
                capabilities: caps.into_row(),
                error: None,
            })
        }
        Err(err) => {
            let probe = crate::agent::probe::EndpointProbe {
                reachable: false,
                models: Vec::new(),
                queried_model_found: false,
                detail: err.to_string(),
            };
            let changed = apply_probe_results_to_capabilities(&mut caps, &probe, &dry_run);
            if changed {
                let row = caps.clone().into_row();
                let _ = state.local_db.upsert_provider_capabilities(&row);
            }
            Ok(ProbeProviderResult {
                probe,
                dry_run,
                capabilities: caps.into_row(),
                error: Some(err.to_string()),
            })
        }
    }
}

fn apply_probe_results_to_capabilities(
    caps: &mut crate::agent::capabilities::ProviderCapabilities,
    probe: &crate::agent::probe::EndpointProbe,
    dry_run: &crate::agent::probe::CapabilityDryRunReport,
) -> bool {
    if caps.source == crate::agent::capabilities::CapabilitySource::UserOverride {
        return false;
    }
    if dry_run.verified {
        if let Some(vision) = dry_run.vision {
            caps.vision = vision;
        }
        if let Some(tool_calls) = dry_run.tool_calls {
            caps.tool_calls = tool_calls;
        }
        if let Some(json_mode) = dry_run.json_mode {
            caps.json_mode = json_mode;
        }
        caps.source = crate::agent::capabilities::CapabilitySource::Verified;
        return true;
    }
    if probe.reachable && probe.queried_model_found {
        caps.source = crate::agent::capabilities::CapabilitySource::Probed;
        return true;
    }
    false
}

fn default_base_url_for(provider_id: &str, protocol: &str) -> String {
    match (provider_id, protocol) {
        ("openai", _) => "https://api.openai.com/v1".to_string(),
        ("anthropic", _) => "https://api.anthropic.com/v1".to_string(),
        ("deepseek", _) => "https://api.deepseek.com/v1".to_string(),
        (_, "ollama") => "http://localhost:11434".to_string(),
        _ => String::new(),
    }
}

#[derive(Debug, serde::Serialize)]
pub struct InitAgentRulesResult {
    pub scope: String,
    pub path: String,
    pub created: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct GlobalAgentRulesRecord {
    pub path: String,
    pub content: String,
}

#[tauri::command]
pub async fn init_agent_rules(
    project_root: Option<String>,
) -> Result<InitAgentRulesResult, String> {
    let project_root = project_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (scope, path) = if let Some(root) = project_root {
        ("project", std::path::PathBuf::from(root).join("atlas.md"))
    } else {
        ("global", crate::agent::skills::global_atlas_path())
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let should_write = match std::fs::metadata(&path) {
        Ok(metadata) => metadata.len() == 0,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => return Err(error.to_string()),
    };

    if should_write {
        std::fs::write(
            &path,
            "# Atlas 规则\n\n在这里写 Atlas 处理任务时需要长期遵守的偏好、项目约定和边界。\n",
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(InitAgentRulesResult {
        scope: scope.to_string(),
        path: path.to_string_lossy().to_string(),
        created: should_write,
    })
}

#[tauri::command]
pub async fn read_global_agent_rules() -> Result<GlobalAgentRulesRecord, String> {
    let path = crate::agent::skills::global_atlas_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.to_string()),
    };
    Ok(GlobalAgentRulesRecord {
        path: path.to_string_lossy().to_string(),
        content,
    })
}

#[tauri::command]
pub async fn save_global_agent_rules(content: String) -> Result<GlobalAgentRulesRecord, String> {
    let path = crate::agent::skills::global_atlas_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, content.as_bytes()).map_err(|error| error.to_string())?;
    Ok(GlobalAgentRulesRecord {
        path: path.to_string_lossy().to_string(),
        content,
    })
}

#[derive(Debug, serde::Serialize)]
pub struct CodeReviewCommandRules {
    pub path: String,
    pub content: String,
    pub warnings: Vec<String>,
}

const BUILTIN_CODE_REVIEW_COMMAND_RULES: &str =
    include_str!("../../../ATLAS_CODE_REVIEW_COMMAND.md");

#[tauri::command]
pub async fn get_code_review_command_rules(
    project_root: Option<String>,
) -> Result<CodeReviewCommandRules, String> {
    let file_name = "ATLAS_CODE_REVIEW_COMMAND.md";
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    let mut warnings = Vec::new();

    if let Some(root) = project_root
        .as_deref()
        .map(str::trim)
        .filter(|root| !root.is_empty())
    {
        candidates.push(std::path::PathBuf::from(root).join(file_name));
    }

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(file_name));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(file_name));
        }
    }

    for path in candidates {
        if path.is_file() {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    return Ok(CodeReviewCommandRules {
                        path: path.to_string_lossy().to_string(),
                        content,
                        warnings,
                    });
                }
                Err(error) => {
                    warnings.push(format!("读取 {} 失败：{}", path.to_string_lossy(), error))
                }
            }
        }
    }

    Ok(CodeReviewCommandRules {
        path: format!("builtin:{file_name}"),
        content: BUILTIN_CODE_REVIEW_COMMAND_RULES.to_string(),
        warnings,
    })
}

#[tauri::command]
pub async fn get_memories(state: State<'_, AppState>) -> Result<Vec<MemoryRecord>, String> {
    state.local_db.list_memories().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_memory(
    text: String,
    source: Option<String>,
    state: State<'_, AppState>,
) -> Result<MemoryRecord, String> {
    state
        .local_db
        .add_memory(&text, source.as_deref().unwrap_or("manual"))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_memory(
    id: String,
    text: Option<String>,
    enabled: Option<bool>,
    state: State<'_, AppState>,
) -> Result<MemoryRecord, String> {
    state
        .local_db
        .update_memory(&id, text, enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_memory(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.local_db.delete_memory(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_memories(state: State<'_, AppState>) -> Result<(), String> {
    state.local_db.clear_memories().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_profile(state: State<'_, AppState>) -> Result<ProfileRecord, String> {
    state.local_db.get_profile().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_profile(
    profile: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<ProfileRecord, String> {
    state
        .local_db
        .save_profile(profile)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_personality_test(
    state: State<'_, AppState>,
) -> Result<PersonalityProgressRecord, String> {
    let progress = serde_json::json!({
        "status": "in_progress",
        "answers": [],
        "startedAt": chrono::Utc::now().to_rfc3339()
    });
    state
        .local_db
        .save_personality_progress(progress)
        .map_err(|e| e.to_string())
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalityQuestion {
    pub id: String,
    pub dimension: String,
    pub text: String,
    pub options: Vec<PersonalityOption>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalityOption {
    pub label: String,
    pub value: i32,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalityQuestionSet {
    pub mode: String,
    pub title: String,
    pub estimated_minutes: u32,
    pub questions: Vec<PersonalityQuestion>,
}

#[tauri::command]
pub async fn get_personality_questions(mode: String) -> Result<PersonalityQuestionSet, String> {
    Ok(personality_questions(&mode))
}

#[tauri::command]
pub async fn get_personality_progress(
    state: State<'_, AppState>,
) -> Result<PersonalityProgressRecord, String> {
    state
        .local_db
        .get_personality_progress()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_personality_progress(
    progress: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<PersonalityProgressRecord, String> {
    state
        .local_db
        .save_personality_progress(progress)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn complete_personality_test(
    answers: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<ProfileRecord, String> {
    state
        .local_db
        .complete_personality_test(answers)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn log_activity_event(
    payload: LogActivityEventPayload,
    state: State<'_, AppState>,
) -> Result<ActivityEvent, String> {
    state
        .local_db
        .log_activity_event(payload)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_recent_activity_events(
    date: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<ActivityEvent>, String> {
    state
        .local_db
        .recent_activity_events(date.as_deref(), limit.unwrap_or(20))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_browser_audit_events(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<BrowserAutomationAuditEvent>, String> {
    list_browser_audit_events(&state.local_db, limit.unwrap_or(80))
}

#[tauri::command]
pub async fn get_artifacts(
    session_id: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<ArtifactRecord>, String> {
    state
        .local_db
        .recent_artifacts(session_id.as_deref(), limit.unwrap_or(80))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn prepare_file_write(
    path: String,
    content: String,
    reason: Option<String>,
    state: State<'_, AppState>,
) -> Result<FileWritePreview, String> {
    let path = tools::fs_scope::allowed_new_path(&path).map_err(|error| error.to_string())?;
    state
        .local_db
        .prepare_file_write(
            path,
            content,
            reason.unwrap_or_else(|| "Atlas 准备写入本地文件。".to_string()),
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn confirm_file_write(
    id: String,
    edited_text: Option<String>,
    session_id: Option<String>,
    run_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<FileWritePreview, String> {
    let should_record_artifact = session_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || run_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    let artifact_scope = if should_record_artifact {
        Some(
            state
                .local_db
                .validate_artifact_scope(session_id.as_deref(), run_id.as_deref())
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let preview = state
        .local_db
        .confirm_pending_file_write(&id, edited_text.as_deref())
        .map_err(|e| e.to_string())?;
    if let Some((session_id, run_id)) = artifact_scope {
        let title = Path::new(&preview.target_path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("文件")
            .to_string();
        state
            .local_db
            .record_artifact(RecordArtifactPayload {
                session_id,
                run_id,
                kind: "file".to_string(),
                title,
                path: Some(preview.target_path.clone()),
                operation: preview.operation.clone(),
                status: "written".to_string(),
                summary: format!("已写入文件：{}", preview.target_path),
                metadata: serde_json::json!({
                    "pendingWriteId": id,
                    "contentSize": preview.content_size,
                    "hasDiff": preview.diff.is_some(),
                    "source": "confirm_file_write"
                }),
            })
            .map_err(|e| format!("文件已写入，但记录文件产物失败：{e}"))?;
    }
    Ok(preview)
}

#[tauri::command]
pub async fn reject_file_write(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state
        .local_db
        .reject_pending_file_write(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_approved_command(
    id: String,
    state: State<'_, AppState>,
    window: Window,
) -> Result<tools::CommandExecutionResult, String> {
    let run_id = state
        .local_db
        .find_run_id_for_pending_command(&id)
        .map_err(|e| e.to_string())?;
    let session_id = run_id
        .as_deref()
        .and_then(|run_id| state.local_db.get_agent_run(run_id).ok().flatten())
        .and_then(|run| run.session_id);
    let pending = state
        .local_db
        .confirm_pending_command(&id)
        .map_err(|e| e.to_string())?;
    let command = pending.command.clone();
    let cwd = pending.cwd.clone();
    let config = state.config.lock().await.clone();
    let project_roots = session_id
        .as_deref()
        .and_then(|session_id| {
            state
                .local_db
                .session_project_root(session_id)
                .ok()
                .flatten()
        })
        .into_iter()
        .collect::<Vec<PathBuf>>();
    let command_isolation =
        tools::CommandIsolationPolicy::from_config(&config.execution, &project_roots);
    let command_display = mask_command_event_text(&command);
    let cwd_display = mask_command_event_text(&cwd);
    let operation_id = format!("approved-command-{id}");
    let event_run_id = run_id
        .clone()
        .unwrap_or_else(|| format!("pending-command-{id}"));
    let event_session_id = session_id.clone();
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let event_window = window.clone();
    let event_handle = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            event_window
                .emit(
                    "agent-event",
                    serde_json::json!({
                        "sessionId": event_session_id.clone(),
                        "runId": event_run_id.clone(),
                        "event": event
                    }),
                )
                .ok();
        }
    });
    let _ = event_tx
        .send(AgentEvent::OperationStarted {
            operation_id: operation_id.clone(),
            tool_name: "run_command".to_string(),
            label: "正在运行已确认的命令".to_string(),
            detail: Some(format!("{cwd_display}\n{command_display}")),
            target: Some(cwd_display.clone()),
            command: Some(command_display.clone()),
        })
        .await;
    let result = tools::execute_shell_command_streaming_with_policy(
        &command,
        Some(&cwd),
        std::time::Duration::from_secs(120),
        Some(event_tx.clone()),
        operation_id.clone(),
        &command_isolation,
    )
    .await;
    match &result {
        Ok(result) => {
            let exit = if result.timed_out {
                "超时".to_string()
            } else {
                result
                    .exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "未知".to_string())
            };
            let _ = event_tx
                .send(AgentEvent::OperationFinished {
                    operation_id: operation_id.clone(),
                    status: if result.exit_code == Some(0) {
                        "success".to_string()
                    } else {
                        "warning".to_string()
                    },
                    summary: format!("命令运行结束，退出码：{exit}。"),
                })
                .await;
        }
        Err(error) => {
            let _ = event_tx
                .send(AgentEvent::OperationFailed {
                    operation_id: operation_id.clone(),
                    summary: format!("命令运行失败：{error}"),
                })
                .await;
        }
    }
    drop(event_tx);
    let _ = event_handle.await;
    match result {
        Ok(result) => {
            if let Some(run_id) = run_id.as_deref() {
                let exit = if result.timed_out {
                    "超时".to_string()
                } else {
                    result
                        .exit_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "未知".to_string())
                };
                let _ = state.local_db.append_agent_run_step(
                    run_id,
                    "command",
                    "finished",
                    &format!("用户确认后运行命令，退出码：{exit}。"),
                    serde_json::json!({
                        "pendingCommandId": id,
                        "command": pending.command,
                        "cwd": pending.cwd,
                    }),
                    serde_json::json!({ "commandResult": result.clone() }),
                );
            }
            if let Some(session_id) = session_id.as_deref() {
                let exit = if result.timed_out {
                    "超时".to_string()
                } else {
                    result
                        .exit_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "未知".to_string())
                };
                persist_pending_command_resolution_message(
                    &state,
                    session_id,
                    &id,
                    serde_json::json!({
                        "status": if result.exit_code == Some(0) { "success" } else { "warning" },
                        "summary": format!("用户确认后命令运行结束，退出码：{exit}。"),
                        "data": {
                            "commandResult": result.clone(),
                            "pendingCommandId": id.clone(),
                            "confirmed": true
                        },
                        "next_actions": [],
                        "recoverable": result.exit_code != Some(0)
                    }),
                );
            }
            Ok(result)
        }
        Err(error) => {
            if let Some(run_id) = run_id.as_deref() {
                let _ = state.local_db.append_agent_run_step(
                    run_id,
                    "command",
                    "failed",
                    &format!("用户确认后运行命令失败：{error}"),
                    serde_json::json!({
                        "pendingCommandId": id,
                        "command": pending.command,
                        "cwd": pending.cwd,
                    }),
                    serde_json::json!({ "error": error.to_string() }),
                );
            }
            if let Some(session_id) = session_id.as_deref() {
                persist_pending_command_resolution_message(
                    &state,
                    session_id,
                    &id,
                    serde_json::json!({
                        "status": "error",
                        "summary": format!("用户确认后命令运行失败：{error}"),
                        "data": {
                            "pendingCommandId": id.clone(),
                            "confirmed": true,
                            "error": error.to_string()
                        },
                        "next_actions": ["查看错误信息后重新决定是否运行。"],
                        "recoverable": true
                    }),
                );
            }
            Err(error.to_string())
        }
    }
}

#[tauri::command]
pub async fn reject_pending_command(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let run_id = state
        .local_db
        .find_run_id_for_pending_command(&id)
        .map_err(|e| e.to_string())?;
    let session_id = run_id
        .as_deref()
        .and_then(|run_id| state.local_db.get_agent_run(run_id).ok().flatten())
        .and_then(|run| run.session_id);
    let pending = state
        .local_db
        .reject_pending_command(&id)
        .map_err(|e| e.to_string())?;
    if let Some(run_id) = run_id.as_deref() {
        let _ = state.local_db.append_agent_run_step(
            run_id,
            "approval",
            "rejected",
            "用户拒绝运行命令，命令没有执行。",
            serde_json::json!({ "pendingCommandId": id }),
            serde_json::json!({ "approved": false }),
        );
    }
    if let Some(session_id) = session_id.as_deref() {
        persist_pending_command_resolution_message(
            &state,
            session_id,
            &id,
            serde_json::json!({
                "status": "warning",
                "summary": "用户拒绝运行命令，命令没有执行。",
                "data": {
                    "pendingCommandId": id.clone(),
                    "command": pending.command,
                    "cwd": pending.cwd,
                    "confirmed": false
                },
                "next_actions": [],
                "recoverable": true
            }),
        );
    }
    Ok(())
}

fn mask_command_event_text(text: &str) -> String {
    crate::tools::secret_scan::scan(
        text,
        crate::tools::secret_scan::SecretLocation::Log,
        crate::tools::secret_scan::SecretAction::Masked,
    )
    .text
}

fn persist_pending_command_resolution_message(
    state: &State<'_, AppState>,
    session_id: &str,
    pending_command_id: &str,
    payload: serde_json::Value,
) {
    let content = serde_json::to_string(&payload).unwrap_or_else(|_| {
        "{\"status\":\"warning\",\"summary\":\"命令确认状态已更新。\",\"data\":{},\"next_actions\":[],\"recoverable\":true}".to_string()
    });
    let _ = state.local_db.save_message(
        session_id,
        SaveMessagePayload {
            id: Some(pending_command_message_id(pending_command_id)),
            role: "tool".to_string(),
            content,
            created_at: None,
            metadata: serde_json::json!({
                "source": "agent_pending_command_resolution",
                "pendingCommandId": pending_command_id
            }),
        },
    );
}

#[tauri::command]
pub async fn get_app_state(
    key: String,
    state: State<'_, AppState>,
) -> Result<Option<serde_json::Value>, String> {
    state
        .local_db
        .get_app_state(&key)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_app_state(
    key: String,
    value: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .local_db
        .set_app_state(&key, value)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_personality_onboarding_state(
    state: State<'_, AppState>,
) -> Result<Option<serde_json::Value>, String> {
    state
        .local_db
        .get_app_state("personality_onboarding")
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_personality_onboarding_state(
    value: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .local_db
        .set_app_state("personality_onboarding", value)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_local_data(state: State<'_, AppState>) -> Result<String, String> {
    state
        .local_db
        .write_export_file()
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reset_local_data(
    options: ResetLocalDataOptions,
    state: State<'_, AppState>,
) -> Result<ResetLocalDataSummary, String> {
    if !options.sessions && !options.memories && !options.profile && !options.app_state {
        return Err("请选择至少一类要重置的本地数据。模型配置和 API Key 不会被清理。".to_string());
    }
    state
        .local_db
        .reset_local_data(options)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_local_db_health(state: State<'_, AppState>) -> Result<LocalDbHealth, String> {
    state.local_db.health().map_err(|e| e.to_string())
}

fn canonical_project_folder(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("请选择项目文件夹。".to_string());
    }
    let path = Path::new(trimmed);
    if !path.exists() || !path.is_dir() {
        return Err("请选择一个存在的文件夹。".to_string());
    }
    path.canonicalize().map_err(|e| e.to_string())
}

fn default_project_parent() -> Result<PathBuf, String> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|e| e.to_string())?);
    let documents = home.join("Documents");
    let base = if documents.exists() { documents } else { home };
    Ok(base.join("Atlas Projects"))
}

fn safe_project_folder_name(title: &str) -> String {
    let mut out = String::new();
    let mut last_was_separator = false;
    for ch in title.trim().chars() {
        let allowed = ch.is_alphanumeric() || ch == ' ' || ch == '-' || ch == '_';
        if allowed {
            out.push(ch);
            last_was_separator = ch == ' ' || ch == '-' || ch == '_';
        } else if !last_was_separator {
            out.push('-');
            last_was_separator = true;
        }
    }
    let cleaned = out
        .trim_matches(|ch| ch == ' ' || ch == '-' || ch == '_')
        .to_string();
    if cleaned.is_empty() {
        "Atlas Project".to_string()
    } else {
        cleaned
    }
}

fn open_folder_in_file_manager(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("explorer.exe");
        command.arg(path);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(path);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path);
        command
    };

    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn personality_questions(mode: &str) -> PersonalityQuestionSet {
    let (title, estimated_minutes, count) = match mode {
        "deep" => ("深度版", 15, 72),
        "standard" => ("标准版", 8, 36),
        _ => ("快速版", 3, 12),
    };
    let options = vec![
        PersonalityOption {
            label: "很不同意".to_string(),
            value: 1,
        },
        PersonalityOption {
            label: "有点不同意".to_string(),
            value: 2,
        },
        PersonalityOption {
            label: "中立".to_string(),
            value: 3,
        },
        PersonalityOption {
            label: "比较同意".to_string(),
            value: 4,
        },
        PersonalityOption {
            label: "很同意".to_string(),
            value: 5,
        },
    ];
    let questions = question_bank()
        .into_iter()
        .take(count)
        .enumerate()
        .map(|(index, (dimension, text))| PersonalityQuestion {
            id: format!("{}_{}", mode, index + 1),
            dimension: dimension.to_string(),
            text: text.to_string(),
            options: options.clone(),
        })
        .collect();
    PersonalityQuestionSet {
        mode: mode.to_string(),
        title: title.to_string(),
        estimated_minutes,
        questions,
    }
}

fn question_bank() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "energy",
            "我希望 Atlas 在我需要独处或专注时少打扰，只在必要时提醒。",
        ),
        (
            "perception",
            "我更希望 Atlas 先给具体事实、文件路径、数据和可验证依据。",
        ),
        (
            "decision",
            "做选择时，我更希望 Atlas 先分析利弊、风险和执行成本。",
        ),
        (
            "execution",
            "我更喜欢 Atlas 帮我把事情整理成明确计划、清单和收尾动作。",
        ),
        (
            "supportMode",
            "我状态不好时，希望 Atlas 先接住情绪，再问我要不要进入解决方案。",
        ),
        (
            "proactivity",
            "我希望 Atlas 主动指出可能被我忽略的下一步，但要说明原因。",
        ),
        ("verbosity", "复杂问题里，我愿意看更长但结构清楚的解释。"),
        (
            "boundary",
            "我能接受 Atlas 温和但明确地指出问题，而不是只顺着我说。",
        ),
        (
            "precision",
            "不确定时，我希望 Atlas 直接说不确定，并给出验证办法。",
        ),
        (
            "focus",
            "我希望 Atlas 在信息很多时帮我压缩重点，而不是继续扩写。",
        ),
        (
            "creativity",
            "我希望 Atlas 在产品、写作或设计问题上给出更有想象力的方案。",
        ),
        (
            "privacy",
            "涉及隐私、文件写入、长期记忆或网络请求时，我希望 Atlas 总是先确认。",
        ),
        (
            "energy",
            "和 Atlas 交流时，我更喜欢低噪声、短回合、能自己安静推进的节奏。",
        ),
        (
            "perception",
            "遇到新工具时，我通常想先看真实例子，再决定是否深入原理。",
        ),
        (
            "decision",
            "如果方案会影响别人，我希望 Atlas 帮我考虑对方感受和沟通方式。",
        ),
        (
            "execution",
            "计划变化时，我希望 Atlas 能快速调整，而不是强行维持原计划。",
        ),
        (
            "supportMode",
            "当我只是想说说时，我不希望 Atlas 立刻给一串建议。",
        ),
        (
            "proactivity",
            "长期项目里，我希望 Atlas 偶尔帮我回看未完成事项。",
        ),
        (
            "verbosity",
            "当我问“怎么做”时，我希望直接拿到操作步骤和检查点。",
        ),
        (
            "boundary",
            "如果功能没接好，我希望 Atlas 明确承认，而不是用漂亮话糊弄。",
        ),
        (
            "precision",
            "涉及代码、配置或测试失败时，我希望 Atlas 给出可复现步骤。",
        ),
        (
            "focus",
            "我容易被细节带偏，希望 Atlas 帮我拉回当前最重要的问题。",
        ),
        (
            "creativity",
            "我喜欢 Atlas 提供一个稳妥方案，再提供一个更大胆的备选。",
        ),
        (
            "privacy",
            "我希望 Atlas 清楚说明它用了哪些本地信号，没有读取哪些内容。",
        ),
        ("energy", "我在多人讨论或信息交换后更容易获得新想法。"),
        (
            "perception",
            "相比参数和细节，我更容易被趋势、可能性和整体方向打动。",
        ),
        ("decision", "我很重视决策中的公平、感受和价值观是否一致。"),
        (
            "execution",
            "我更适合保留弹性，在推进中不断修正目标和方法。",
        ),
        (
            "supportMode",
            "压力很大时，我希望 Atlas 先帮我降低噪声，再推进事情。",
        ),
        (
            "proactivity",
            "当 Atlas 发现明显风险时，我希望它主动提醒我，而不是等我问。",
        ),
        (
            "verbosity",
            "我希望 Atlas 能根据问题重要程度自动调整回答长度。",
        ),
        (
            "boundary",
            "我不希望 Atlas 为了讨好我而假装同意不合理的判断。",
        ),
        (
            "precision",
            "我希望 Atlas 引用外部信息时说明来源、时间和不确定性。",
        ),
        (
            "focus",
            "长对话结束后，我希望 Atlas 能总结成可以继续执行的摘要。",
        ),
        (
            "creativity",
            "我希望 Atlas 在审美和体验问题上有自己的判断，不只是列选项。",
        ),
        (
            "privacy",
            "我希望可以随时查看、暂停和删除 Atlas 的画像、记忆和活动记录。",
        ),
        ("energy", "我在低能量时更需要轻量建议，而不是完整长计划。"),
        (
            "perception",
            "面对模糊想法时，我希望 Atlas 先帮我画出整体结构。",
        ),
        (
            "decision",
            "出现冲突时，我希望 Atlas 先帮我把事实和情绪分开。",
        ),
        (
            "execution",
            "我喜欢先跑通一个可用版本，再慢慢打磨体验和细节。",
        ),
        (
            "supportMode",
            "我希望 Atlas 能分辨我是在求安慰、求建议还是求执行。",
        ),
        (
            "proactivity",
            "我希望 Atlas 只在有明确理由时主动出现，不要制造存在感。",
        ),
        ("verbosity", "我不喜欢重复解释已经明确的上下文。"),
        (
            "boundary",
            "我希望 Atlas 能提醒我哪些能力尚未接入，哪些事情不该做。",
        ),
        (
            "precision",
            "我对测试结果和失败原因很敏感，希望 Atlas 不要只说“已优化”。",
        ),
        (
            "focus",
            "我希望 Atlas 能主动指出当前体验里最影响使用的瓶颈。",
        ),
        (
            "creativity",
            "我喜欢普通工具带一点个人感，但不能牺牲清晰和稳定。",
        ),
        (
            "privacy",
            "如果 Atlas 要保存一条关于我的长期偏好，我希望先看到建议内容。",
        ),
        ("energy", "我更容易在夜晚进入深度思考或创作状态。"),
        (
            "perception",
            "我更相信能落到文件、任务、记录和结果上的信息。",
        ),
        (
            "decision",
            "我希望 Atlas 在给建议时同时考虑效率和人的感受。",
        ),
        (
            "execution",
            "我做事常常需要先探索一段时间，再确定最终计划。",
        ),
        (
            "supportMode",
            "我卡住时，希望 Atlas 给我一个很小但能启动的下一步。",
        ),
        (
            "proactivity",
            "我希望 Atlas 主动发现无效按钮、假功能和体验断点。",
        ),
        ("verbosity", "重要结论应该短，背景解释可以放在后面。"),
        (
            "boundary",
            "我希望 Atlas 保护我的注意力，不随便扩展无关内容。",
        ),
        (
            "precision",
            "我希望 Atlas 在写代码时优先稳定性、可读性和可验证性。",
        ),
        (
            "focus",
            "我希望 Atlas 帮我把想法整理成清晰计划，而不是只陪我发散。",
        ),
        (
            "creativity",
            "我希望 Atlas 能帮我把普通表达打磨得更有质感。",
        ),
        (
            "privacy",
            "我能接受更强的本地能力，但前提是权限、原因和记录都透明。",
        ),
        (
            "energy",
            "我希望 Atlas 记住什么时候适合深聊，什么时候适合简短执行。",
        ),
        (
            "perception",
            "如果一个想法还没有数据支撑，我仍愿意先探索它的可能性。",
        ),
        (
            "decision",
            "做重要决定前，我希望 Atlas 提醒我可能被忽略的人际影响。",
        ),
        (
            "execution",
            "我希望 Atlas 把计划当作可调整草稿，而不是一旦确定就不变。",
        ),
        (
            "supportMode",
            "我希望 Atlas 的关心克制真实，不制造负罪感或依赖感。",
        ),
        (
            "proactivity",
            "我希望 Atlas 主动建议下一步，但不能自动执行关键动作。",
        ),
        (
            "verbosity",
            "我希望 Atlas 能把长内容拆成层次，而不是堆成一大段。",
        ),
        (
            "boundary",
            "当我要求越权读取、写入或推断时，Atlas 应该拒绝并解释边界。",
        ),
        (
            "precision",
            "我希望 Atlas 区分事实、推测和建议，不把窗口标题当确定事实。",
        ),
        (
            "focus",
            "我希望 Atlas 在多任务时帮我确定优先级，而不是把所有事都展开。",
        ),
        (
            "creativity",
            "我希望 Atlas 偶尔提供出乎意料但可落地的设计方向。",
        ),
        (
            "privacy",
            "我希望 Atlas 的个性化只服务于我的使用体验，不把我固定成某种标签。",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn personality_question_sets_have_unique_text() {
        for mode in ["quick", "standard", "deep"] {
            let set = personality_questions(mode);
            let unique = set
                .questions
                .iter()
                .map(|question| question.text.as_str())
                .collect::<HashSet<_>>();
            assert_eq!(
                unique.len(),
                set.questions.len(),
                "{mode} has duplicated questions"
            );
        }
    }

    #[test]
    fn personality_question_counts_match_modes() {
        assert_eq!(personality_questions("quick").questions.len(), 12);
        assert_eq!(personality_questions("standard").questions.len(), 36);
        assert_eq!(personality_questions("deep").questions.len(), 72);
    }

    #[test]
    fn personality_question_labels_are_readable_chinese() {
        let set = personality_questions("quick");
        assert_eq!(set.title, "快速版");
        assert_eq!(set.questions[0].options[0].label, "很不同意");
        assert!(set.questions[0].text.contains("Atlas"));
    }

    #[test]
    fn dry_run_verified_result_updates_capabilities() {
        let mut caps = crate::agent::capabilities::ProviderCapabilities {
            provider_id: "openai".to_string(),
            model: "text-only".to_string(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 128_000,
            source: crate::agent::capabilities::CapabilitySource::Builtin,
        };
        let probe = crate::agent::probe::EndpointProbe {
            reachable: false,
            models: Vec::new(),
            queried_model_found: false,
            detail: "model list unavailable".to_string(),
        };
        let dry_run = crate::agent::probe::CapabilityDryRunReport {
            attempted: true,
            protocol: "openai-compatible".to_string(),
            verified: true,
            vision: Some(false),
            tool_calls: Some(true),
            json_mode: Some(true),
            checks: Vec::new(),
        };

        assert!(apply_probe_results_to_capabilities(
            &mut caps, &probe, &dry_run
        ));
        assert_eq!(
            caps.source,
            crate::agent::capabilities::CapabilitySource::Verified
        );
        assert!(!caps.vision);
        assert!(caps.tool_calls);
        assert!(caps.json_mode);
    }

    #[test]
    fn dry_run_does_not_overwrite_user_override() {
        let mut caps = crate::agent::capabilities::ProviderCapabilities {
            provider_id: "openai".to_string(),
            model: "manual".to_string(),
            vision: false,
            tool_calls: false,
            json_mode: false,
            max_context: 8_192,
            source: crate::agent::capabilities::CapabilitySource::UserOverride,
        };
        let probe = crate::agent::probe::EndpointProbe {
            reachable: true,
            models: vec!["manual".to_string()],
            queried_model_found: true,
            detail: "found".to_string(),
        };
        let dry_run = crate::agent::probe::CapabilityDryRunReport {
            attempted: true,
            protocol: "openai-compatible".to_string(),
            verified: true,
            vision: Some(true),
            tool_calls: Some(true),
            json_mode: Some(true),
            checks: Vec::new(),
        };

        assert!(!apply_probe_results_to_capabilities(
            &mut caps, &probe, &dry_run
        ));
        assert_eq!(
            caps.source,
            crate::agent::capabilities::CapabilitySource::UserOverride
        );
        assert!(!caps.vision);
        assert!(!caps.tool_calls);
        assert!(!caps.json_mode);
    }
}
