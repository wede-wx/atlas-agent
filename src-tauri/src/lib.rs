pub mod agent;
pub mod browser_automation;
mod commands;
pub mod config;
mod env;
pub mod feishu;
pub mod mcp;
pub mod storage;
pub mod tools;
pub mod web;

#[cfg(test)]
mod security_attack_suite;

use agent::{anthropic::AnthropicClient, openai::OpenAIClient, AgentGuidanceMessage, LLMClient};
use config::{Config, ConfigError};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use storage::LocalDb;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tools::ToolRegistry;

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub cancel_tokens: Arc<Mutex<HashMap<String, CancellationToken>>>,
    pub active_session_runs: Arc<Mutex<HashMap<String, String>>>,
    pub run_guidance: Arc<Mutex<HashMap<String, Vec<AgentGuidanceMessage>>>>,
    /// P1-2: per-run 暂停句柄,按 run_id 索引(命令层置位 pause/resume,agent 循环只读)。
    pub pause_registry: crate::agent::RunPauseRegistry,
    pub feishu_callback_server: Arc<Mutex<Option<feishu::FeishuCallbackServer>>>,
    pub feishu_tunnel_process: Arc<Mutex<Option<feishu::FeishuTunnelProcess>>>,
    pub local_db: LocalDb,
}

pub fn create_tool_registry(local_db: LocalDb) -> ToolRegistry {
    create_tool_registry_with_runtime(
        local_db,
        None,
        None,
        None,
        None,
        crate::tools::execution_isolation::ExecutionIsolationConfig::default(),
    )
}

pub fn create_runtime_tool_registry(
    local_db: LocalDb,
    cancel_tokens: Arc<Mutex<HashMap<String, CancellationToken>>>,
    current_session_id: Option<String>,
    current_run_id: Option<String>,
    project_root: Option<PathBuf>,
    execution_isolation: crate::tools::execution_isolation::ExecutionIsolationConfig,
) -> ToolRegistry {
    create_tool_registry_with_runtime(
        local_db,
        Some(cancel_tokens),
        current_session_id,
        current_run_id,
        project_root,
        execution_isolation,
    )
}

fn create_tool_registry_with_runtime(
    local_db: LocalDb,
    cancel_tokens: Option<Arc<Mutex<HashMap<String, CancellationToken>>>>,
    current_session_id: Option<String>,
    current_run_id: Option<String>,
    project_root: Option<PathBuf>,
    execution_isolation: crate::tools::execution_isolation::ExecutionIsolationConfig,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    let project_roots = project_root.into_iter().collect::<Vec<_>>();
    let command_isolation =
        crate::tools::CommandIsolationPolicy::from_config(&execution_isolation, &project_roots);
    registry.register(Box::new(tools::ReadFileTool::new(project_roots.clone())));
    registry.register(Box::new(tools::ListDirectoryTool::new(
        project_roots.clone(),
    )));
    registry.register(Box::new(tools::SearchFilesTool::new(project_roots.clone())));
    registry.register(Box::new(tools::FileInfoTool::new(project_roots.clone())));
    registry.register(Box::new(tools::WriteFileTool::new_with_roots(
        local_db.clone(),
        project_roots.clone(),
        current_session_id.clone(),
    )));
    registry.register(Box::new(tools::EditFileTool::new_with_roots(
        local_db.clone(),
        project_roots.clone(),
        current_session_id.clone(),
    )));
    let project_root_for_verify = project_roots.first().cloned();
    registry.register(Box::new(tools::CreateDirectoryTool::new(
        project_roots.clone(),
    )));
    registry.register(Box::new(tools::ResetTaskTool::new(
        local_db.clone(),
        current_session_id.clone(),
    )));
    registry.register(Box::new(tools::PurgeRunCheckpointsTool::new(
        local_db.clone(),
        current_session_id.clone(),
    )));
    registry.register(Box::new(tools::GitStatusTool));
    registry.register(Box::new(tools::GitDiffTool));
    registry.register(Box::new(tools::GitLogTool));
    registry.register(Box::new(tools::GitShowTool));
    registry.register(Box::new(tools::GitStageTool::new_with_roots(
        project_roots.clone(),
    )));
    registry.register(Box::new(tools::GitCommitTool::new_with_roots(
        project_roots.clone(),
    )));
    registry.register(Box::new(tools::GitCreateBranchTool::new_with_roots(
        project_roots.clone(),
    )));
    registry.register(Box::new(tools::GitPushTool::new_with_roots(
        project_roots.clone(),
    )));
    registry.register(Box::new(tools::RunVerifyTool::new(
        local_db.clone(),
        project_root_for_verify.clone(),
        current_session_id.clone(),
    )));
    registry.register(Box::new(tools::PrepareCommandTool::new_with_isolation(
        local_db.clone(),
        command_isolation.clone(),
    )));
    registry.register(Box::new(tools::RunCommandTool::new(command_isolation)));
    registry.register(Box::new(match cancel_tokens {
        Some(tokens) => tools::StopRunTool::new(tokens, current_session_id.clone()),
        None => tools::StopRunTool::unavailable(),
    }));
    registry.register(Box::new(tools::SearchWebTool));
    registry.register(Box::new(tools::FetchWebPageTool));
    registry.register(Box::new(tools::OpenWebSearchTool));
    registry.register(Box::new(tools::GetGithubTrendingTool));
    registry.register(Box::new(tools::BrowserAutomationTool::new(
        local_db.clone(),
        current_session_id.clone(),
        current_run_id,
    )));
    registry.register(Box::new(tools::InvokeMcpTool::new(local_db.clone())));
    registry.register(Box::new(tools::CreatePlanTool::new(
        local_db.clone(),
        current_session_id.clone(),
    )));
    registry.register(Box::new(tools::CreatePlanTaskTool::new(
        local_db.clone(),
        current_session_id.clone(),
    )));
    registry.register(Box::new(
        tools::UpdatePlanTaskTool::new(local_db.clone(), current_session_id.clone())
            .with_project_root(project_root_for_verify.clone()),
    ));
    registry.register(Box::new(tools::ListPlanTasksTool::new(
        local_db.clone(),
        current_session_id.clone(),
    )));
    registry.register(Box::new(tools::SetActivePlanTaskTool::new(
        local_db.clone(),
        current_session_id,
    )));
    registry.register(Box::new(tools::InstallPluginPackageTool::new(
        local_db.clone(),
    )));
    registry.register(Box::new(tools::ListPluginPackagesTool::new(
        local_db.clone(),
    )));
    registry.register(Box::new(tools::SetPluginPackageEnabledTool::new(
        local_db.clone(),
    )));
    registry.register(Box::new(tools::InvokePluginCapabilityTool::new(
        local_db.clone(),
    )));
    tools::register_installed_plugin_capabilities(&mut registry, local_db.clone());
    registry.register(Box::new(tools::AddMemoryTool::new(local_db)));
    registry
}

pub fn create_llm_client(
    config: &Config,
    db: Option<&LocalDb>,
) -> Result<Box<dyn LLMClient>, ConfigError> {
    config.validate_for_chat()?;
    let connection = config
        .llm
        .active_connection()
        .ok_or_else(|| ConfigError::MissingApiKey(config.llm.default_provider.clone()))?;
    // M5: vision capability comes from the structured matrix when a DB is
    // available; tests that pass `None` skip the lookup and the client falls
    // through to its "unknown" default (current behavior preserved).
    let resolved_caps = db.and_then(|db| {
        crate::agent::capabilities::resolve_capabilities(
            db,
            &connection.provider_id,
            &connection.model,
        )
        .ok()
    });
    let vision_supported = resolved_caps.as_ref().map(|cap| cap.vision);
    let tool_calls_supported = resolved_caps.as_ref().map(|cap| cap.tool_calls);
    let json_mode_supported = resolved_caps.as_ref().map(|cap| cap.json_mode);
    let tool_protocol_caps = resolved_caps.as_ref().map(|cap| cap.tool_protocol_caps());
    let protocol_vision_supported = tool_protocol_caps.as_ref().map(|caps| {
        !matches!(
            caps.vision_input_format,
            crate::agent::VisionInputFormat::None
        )
    });
    let protocol_tool_calls_supported = tool_protocol_caps
        .as_ref()
        .map(|caps| caps.structured_tool_calls);
    let protocol_json_mode_supported = tool_protocol_caps
        .as_ref()
        .map(|caps| caps.supports_json_response_format);

    // P0-3: provider-API outbound sub-boundary. Validate the effective endpoint
    // before building a client that will POST to it. The provider channel allows
    // loopback (local model servers such as Ollama / LM Studio); only malformed
    // or non-http(s) endpoints are refused. The conversation payload itself is
    // already secret-masked upstream at context injection (P0-1), so it is not
    // re-scanned here.
    if let Some(base_url) = &connection.base_url {
        let normalized = crate::config::normalize_base_url(base_url);
        let decision = config.outbound.evaluate_url(
            crate::tools::outbound::OutboundChannel::ProviderApi,
            &normalized,
            &[],
        );
        crate::tools::outbound::OutboundAudit {
            channel: crate::tools::outbound::OutboundChannel::ProviderApi,
            target: crate::tools::outbound::audit_target_host(&normalized),
            allowed: decision.is_allowed(),
            secret_hits: 0,
            summary: format!(
                "provider={} model={}",
                connection.provider_id, connection.model
            ),
        }
        .emit();
        if let crate::tools::outbound::OutboundDecision::Deny { reason } = decision {
            return Err(ConfigError::OutboundDenied(reason));
        }
    }

    match connection.protocol.as_str() {
        "openai-compatible" => {
            let mut client =
                OpenAIClient::new(connection.api_key.clone(), connection.model.clone())
                    .with_auth_header(connection.auth_header.clone())
                    .with_vision_support(protocol_vision_supported.or(vision_supported))
                    .with_tool_call_support(protocol_tool_calls_supported.or(tool_calls_supported))
                    .with_json_mode_supported(protocol_json_mode_supported.or(json_mode_supported));
            if let Some(base_url) = &connection.base_url {
                client = client.with_base_url(crate::config::normalize_base_url(base_url));
            }
            Ok(Box::new(client))
        }
        "anthropic" => {
            let mut client =
                AnthropicClient::new(connection.api_key.clone(), connection.model.clone())
                    .with_vision_support(protocol_vision_supported.or(vision_supported))
                    .with_tool_call_support(protocol_tool_calls_supported.or(tool_calls_supported));
            if let Some(base_url) = &connection.base_url {
                client = client.with_base_url(crate::config::normalize_base_url(base_url));
            }
            Ok(Box::new(client))
        }
        other => Err(ConfigError::UnsupportedProvider(other.to_string())),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = Config::load().expect("Failed to load config");
    let local_db = LocalDb::open_default().expect("Failed to initialize Atlas local database");
    local_db
        .mark_interrupted_agent_runs()
        .expect("Failed to reconcile interrupted Atlas agent runs");

    // Patch 17 / #18: load hook config from ~/.atlas/hooks.toml.
    agent::hooks::reload_global_from_home();

    // P0-3: prime the outbound network policy from config so every channel
    // (including the zero-config tool boundaries) honors the user's settings.
    crate::tools::outbound::set_active_policy(config.outbound.clone());

    let state = AppState {
        config: Arc::new(Mutex::new(config)),
        cancel_tokens: Arc::new(Mutex::new(HashMap::new())),
        active_session_runs: Arc::new(Mutex::new(HashMap::new())),
        run_guidance: Arc::new(Mutex::new(HashMap::new())),
        pause_registry: Arc::new(Mutex::new(HashMap::new())),
        feishu_callback_server: Arc::new(Mutex::new(None)),
        feishu_tunnel_process: Arc::new(Mutex::new(None)),
        local_db,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .setup(|_app| Ok(()))
        .invoke_handler(tauri::generate_handler![
            commands::agent_chat,
            commands::cancel_agent_chat,
            commands::pause_agent_chat,
            commands::resume_agent_chat,
            commands::agent_chat_v2,
            commands::agent_subagent_chat,
            commands::summarize_session,
            commands::get_context_usage,
            commands::get_agent_skill_metadata,
            commands::get_agent_subagent_metadata,
            commands::get_agent_skill_input,
            commands::get_model_usage_summary,
            commands::get_agent_runs,
            commands::get_agent_run_steps,
            commands::get_agent_run_timeline,
            commands::get_browser_agent_steps,
            commands::create_agent_graph_run,
            commands::get_agent_graph_snapshot,
            commands::pause_agent_graph_run,
            commands::resume_agent_graph_run,
            commands::create_team_run,
            commands::get_team_run_snapshot,
            commands::append_team_message,
            commands::create_handoff_request,
            commands::resolve_handoff_request,
            commands::apply_team_termination,
            commands::schedule_team_execution_plan,
            commands::pause_team_execution,
            commands::resume_team_execution,
            commands::add_knowledge_item,
            commands::search_knowledge,
            commands::delete_knowledge_item,
            commands::ingest_connector_knowledge_items,
            commands::write_agent_memory_event,
            commands::create_workspace_lifecycle,
            commands::get_workspace_lifecycle_snapshot,
            commands::validate_workspace_cwd,
            commands::run_workspace_setup_script,
            commands::validate_workspace_command_binding,
            commands::install_workspace_git_hook,
            commands::export_agent_run_trajectory,
            commands::replay_agent_run_trajectory,
            commands::evaluate_agent_run_trajectory,
            commands::create_external_agent_task,
            commands::get_external_agent_task_mapping,
            commands::get_protocol_compatibility_matrix,
            commands::update_external_agent_task_lifecycle,
            commands::cancel_external_agent_task,
            commands::append_external_agent_task_stream_event,
            commands::list_external_agent_task_stream_events,
            commands::evaluate_plugin_quality_gate,
            commands::get_plugin_eval_registry_entry,
            commands::get_skill_version_registry,
            commands::get_team_preset_permission_report,
            commands::inspect_code_intelligence_report,
            commands::prepare_lsp_session_plan,
            commands::estimate_model_cost,
            commands::estimate_model_text_cost,
            commands::record_model_quality_event,
            commands::get_model_quality_events,
            commands::get_model_route_decisions,
            commands::explain_route_economics,
            commands::get_agent_graph_node_traces,
            commands::enqueue_agent_graph_run,
            commands::abort_queued_agent_graph_run,
            commands::list_agent_graph_queue,
            commands::set_agent_graph_queue_paused,
            commands::get_agent_graph_queue_control,
            commands::get_agent_run_diff,
            commands::get_agent_run_terminal,
            commands::get_agent_run_audit,
            commands::get_agent_run_progress,
            commands::get_agent_status_semantic,
            commands::get_agent_eval_suites,
            commands::score_agent_eval_suite,
            commands::run_agent_eval_suite_verifiers,
            commands::classify_user_intent,
            commands::get_project_snapshot,
            commands::get_agent_tool_audit_events,
            commands::get_agent_permission_decisions,
            commands::resolve_permission_confirmation,
            commands::retry_agent_run,
            commands::start_feishu_callback_server,
            commands::stop_feishu_callback_server,
            commands::get_feishu_callback_status,
            commands::set_feishu_public_url,
            commands::get_feishu_setup_links,
            commands::start_feishu_public_tunnel,
            commands::stop_feishu_public_tunnel,
            commands::get_feishu_tunnel_status,
            commands::get_feishu_received_events,
            commands::ingest_feishu_event_payload,
            commands::get_config,
            commands::save_config,
            commands::delete_model_connection,
            commands::reveal_model_connection_key,
            commands::get_backend_status,
            commands::write_settings_smoke_proof,
            commands::write_settings_persistence_smoke_proof,
            commands::write_settings_domain_smoke_proof,
            commands::write_agent_workbench_smoke_proof,
            commands::check_model_settings,
            commands::list_models,
            commands::search_web,
            commands::open_external_web_search,
            commands::fetch_web_page,
            commands::get_github_trending,
            commands::init_local_db,
            commands::get_sessions,
            commands::list_projects,
            commands::upsert_project,
            commands::create_project_folder,
            commands::rename_project,
            commands::set_project_pinned,
            commands::archive_project,
            commands::delete_project,
            commands::open_project_in_explorer,
            commands::generate_history_report,
            commands::get_archived_sessions,
            commands::search_sessions,
            commands::create_session,
            commands::rename_session,
            commands::delete_session,
            commands::archive_session,
            commands::restore_session,
            commands::set_session_pinned,
            commands::get_messages,
            commands::save_message,
            commands::clear_session_context,
            commands::get_plan_tasks,
            commands::create_plan_task,
            commands::update_plan_task,
            commands::update_plan_task_status,
            commands::archive_plan_task,
            commands::set_active_plan_task,
            commands::waive_plan_task,
            commands::get_plan_change_events,
            commands::scan_plan_task_run_integrity,
            commands::repair_plan_task_run_integrity,
            commands::git_status,
            commands::git_diff,
            commands::git_log,
            commands::git_show,
            commands::git_stage,
            commands::git_commit,
            commands::git_create_branch,
            commands::git_push,
            commands::install_plugin_package,
            commands::list_plugin_packages,
            commands::set_plugin_package_enabled,
            commands::invoke_plugin_capability,
            commands::get_plugin_capability_events,
            commands::list_provider_capabilities,
            commands::resolve_provider_capabilities,
            commands::probe_provider_capabilities,
            commands::reset_provider_capabilities,
            commands::list_capability_audit,
            commands::override_provider_capabilities,
            commands::init_agent_rules,
            commands::read_global_agent_rules,
            commands::save_global_agent_rules,
            commands::get_code_review_command_rules,
            commands::get_mcp_servers,
            commands::save_mcp_server,
            commands::delete_mcp_server,
            commands::set_mcp_server_trust,
            commands::test_mcp_server,
            commands::invoke_mcp_tool,
            commands::get_mcp_audit_events,
            commands::get_memories,
            commands::add_memory,
            commands::update_memory,
            commands::delete_memory,
            commands::clear_memories,
            commands::get_profile,
            commands::save_profile,
            commands::start_personality_test,
            commands::get_personality_questions,
            commands::get_personality_progress,
            commands::save_personality_progress,
            commands::complete_personality_test,
            commands::prepare_file_write,
            commands::confirm_file_write,
            commands::reject_file_write,
            commands::run_approved_command,
            commands::reject_pending_command,
            commands::get_app_state,
            commands::set_app_state,
            commands::get_personality_onboarding_state,
            commands::save_personality_onboarding_state,
            commands::export_local_data,
            commands::reset_local_data,
            commands::get_local_db_health,
            commands::log_activity_event,
            commands::get_recent_activity_events,
            commands::get_browser_audit_events,
            commands::get_artifacts,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        Config, LLMConfig, ModelConnectionConfig, ProviderConfig, TmdbConfig, UiConfig,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn openai_compatible_config(provider: &str, base_url: &str) -> Config {
        Config {
            llm: LLMConfig {
                default_provider: provider.to_string(),
                default_connection_id: Some(format!("{provider}:test")),
                connections: vec![ModelConnectionConfig {
                    id: format!("{provider}:test"),
                    name: provider.to_string(),
                    provider_id: provider.to_string(),
                    route_id: "test".to_string(),
                    protocol: "openai-compatible".to_string(),
                    api_key: "test-key".to_string(),
                    model: "test-model".to_string(),
                    base_url: Some(base_url.to_string()),
                    enabled: true,
                    auth_header: None,
                }],
                openai: Some(ProviderConfig {
                    api_key: "test-key".to_string(),
                    model: "test-model".to_string(),
                    base_url: Some(base_url.to_string()),
                }),
                anthropic: None,
            },
            ui: UiConfig::default(),
            tmdb: TmdbConfig::default(),
            execution: crate::tools::execution_isolation::ExecutionIsolationConfig::default(),
            outbound: Default::default(),
        }
    }

    fn temp_db() -> LocalDb {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        LocalDb::open(std::env::temp_dir().join(format!("atlas_tool_registry_{unique}.db")))
            .unwrap()
    }

    #[test]
    fn creates_openai_client_with_deepseek_base_url() {
        let config = openai_compatible_config("openai", "https://api.deepseek.com/v1");
        assert!(create_llm_client(&config, None).is_ok());
    }

    #[test]
    fn creates_local_openai_compatible_clients() {
        let ollama = openai_compatible_config("ollama", "http://localhost:11434/v1");
        let lmstudio = openai_compatible_config("lmstudio", "http://localhost:1234/v1");

        assert!(create_llm_client(&ollama, None).is_ok());
        assert!(create_llm_client(&lmstudio, None).is_ok());
    }

    #[test]
    fn resolve_capabilities_persists_builtin_on_first_use() {
        let db = temp_db();
        let caps =
            crate::agent::capabilities::resolve_capabilities(&db, "xiaomi-mimo", "mimo-v2.5-pro")
                .unwrap();
        assert!(!caps.vision, "mimo has no vision");
        assert!(caps.tool_calls, "mimo supports tool_calls");
        let row = db
            .get_provider_capabilities("xiaomi-mimo", "mimo-v2.5-pro")
            .unwrap()
            .expect("should have persisted");
        assert_eq!(row.source, "builtin");
        let caps2 =
            crate::agent::capabilities::resolve_capabilities(&db, "xiaomi-mimo", "mimo-v2.5-pro")
                .unwrap();
        assert!(!caps2.vision);
    }

    #[test]
    fn capability_audit_logs_diff_on_overwrite() {
        // T30: upsert with a changed source / boolean should write audit rows.
        let db = temp_db();
        // First call: creates row + writes one "created" audit row.
        let row1 = crate::storage::ProviderCapabilitiesRow {
            provider_id: "openai".into(),
            model: "gpt-4o-mini".into(),
            vision: true,
            tool_calls: true,
            json_mode: true,
            max_context: 128_000,
            source: "builtin".into(),
            updated_at: 0,
        };
        db.upsert_provider_capabilities(&row1).unwrap();
        let audit1 = db
            .list_capability_audit(Some("openai"), Some("gpt-4o-mini"), 50)
            .unwrap();
        assert_eq!(audit1.len(), 1);
        assert_eq!(audit1[0].field, "created");

        // Second call: override vision=false + source=user_override → 2 diffs.
        let mut row2 = row1.clone();
        row2.vision = false;
        row2.source = "user_override".into();
        db.upsert_provider_capabilities(&row2).unwrap();
        let audit2 = db
            .list_capability_audit(Some("openai"), Some("gpt-4o-mini"), 50)
            .unwrap();
        assert_eq!(audit2.len(), 3); // 1 created + 2 diffs
                                     // Most recent first
        let fields: Vec<&str> = audit2.iter().take(2).map(|r| r.field.as_str()).collect();
        assert!(fields.contains(&"vision"));
        assert!(fields.contains(&"source"));
    }

    #[test]
    fn reset_capabilities_drops_rows_and_re_persists_on_next_resolve() {
        // T33: after reset, the next resolve re-writes the builtin row.
        let db = temp_db();
        // Persist via resolve.
        let _ = crate::agent::capabilities::resolve_capabilities(&db, "deepseek", "deepseek-chat")
            .unwrap();
        assert!(db
            .get_provider_capabilities("deepseek", "deepseek-chat")
            .unwrap()
            .is_some());
        let deleted = db.reset_capabilities_for_provider("deepseek").unwrap();
        assert!(deleted >= 1);
        assert!(db
            .get_provider_capabilities("deepseek", "deepseek-chat")
            .unwrap()
            .is_none());
        // Resolve again → re-persists.
        let _ = crate::agent::capabilities::resolve_capabilities(&db, "deepseek", "deepseek-chat")
            .unwrap();
        assert!(db
            .get_provider_capabilities("deepseek", "deepseek-chat")
            .unwrap()
            .is_some());
    }

    #[test]
    fn production_tool_registry_metadata_is_policy_consistent() {
        let registry = create_tool_registry(temp_db());
        let issues = registry.metadata_issues();
        assert!(
            issues.is_empty(),
            "tool metadata issues: {}",
            serde_json::to_string_pretty(&issues).unwrap()
        );
    }

    #[test]
    fn runtime_tool_registry_matches_builtin_skill_tools() {
        let registry = create_tool_registry(temp_db());
        let names = registry
            .list_metadata()
            .into_iter()
            .map(|metadata| metadata.name)
            .collect::<std::collections::BTreeSet<_>>();
        let expected = [
            "read_file",
            "list_directory",
            "search_files",
            "file_info",
            "write_file",
            "edit_file",
            "create_directory",
            "prepare_command",
            "run_command",
            "stop_run",
            "search_web",
            "fetch_web_page",
            "open_web_search",
            "get_github_trending",
            "browser_automation",
            "invoke_mcp_tool",
            "add_memory",
            // M2+M3 plan_tasks runtime additions
            "create_plan",
            "create_plan_task",
            "list_plan_tasks",
            "update_plan_task",
            "set_active_plan_task",
            "reset_task",
            "run_verify",
            "purge_run_checkpoints",
            "git_status",
            "git_diff",
            "git_log",
            "git_show",
            // P3-1 controlled git write tools
            "git_stage",
            "git_commit",
            "git_create_branch",
            "git_push",
            // P3-6 plugin capability packages
            "install_plugin_package",
            "list_plugin_packages",
            "set_plugin_package_enabled",
            "invoke_plugin_capability",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(names, expected);
    }

    #[test]
    fn core_agent_tools_respect_permission_policies() {
        let registry = create_tool_registry(temp_db());
        let plan_names = registry
            .list_schemas_for_policy(&tools::ToolAccessPolicy::Plan)
            .into_iter()
            .map(|schema| schema.name)
            .collect::<Vec<_>>();
        assert!(plan_names.contains(&"read_file".to_string()));
        assert!(plan_names.contains(&"list_directory".to_string()));
        assert!(!plan_names.contains(&"write_file".to_string()));
        assert!(!plan_names.contains(&"run_command".to_string()));
        assert!(!plan_names.contains(&"git_stage".to_string()));
        assert!(!plan_names.contains(&"git_commit".to_string()));
        assert!(!plan_names.contains(&"git_create_branch".to_string()));
        assert!(!plan_names.contains(&"git_push".to_string()));

        let default_names = registry
            .list_schemas_for_policy(&tools::ToolAccessPolicy::Default)
            .into_iter()
            .map(|schema| schema.name)
            .collect::<Vec<_>>();
        assert!(default_names.contains(&"write_file".to_string()));
        assert!(default_names.contains(&"edit_file".to_string()));
        assert!(default_names.contains(&"prepare_command".to_string()));
        assert!(default_names.contains(&"git_stage".to_string()));
        assert!(default_names.contains(&"git_commit".to_string()));
        assert!(default_names.contains(&"git_create_branch".to_string()));
        assert!(default_names.contains(&"git_push".to_string()));
        assert!(!default_names.contains(&"run_command".to_string()));

        let full_names = registry
            .list_schemas_for_policy(&tools::ToolAccessPolicy::FullAccess)
            .into_iter()
            .map(|schema| schema.name)
            .collect::<Vec<_>>();
        assert!(full_names.contains(&"write_file".to_string()));
        assert!(full_names.contains(&"run_command".to_string()));
        assert!(full_names.contains(&"git_stage".to_string()));
        assert!(full_names.contains(&"git_commit".to_string()));
        assert!(full_names.contains(&"git_create_branch".to_string()));
        assert!(full_names.contains(&"git_push".to_string()));
        assert!(!full_names.contains(&"prepare_command".to_string()));
    }
}
