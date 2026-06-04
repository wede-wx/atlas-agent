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
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use storage::LocalDb;
use tauri::{Emitter, LogicalSize, Manager, PhysicalPosition, Runtime};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tools::ToolRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DesktopRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl DesktopRect {
    fn right(self) -> i32 {
        self.x.saturating_add(self.width)
    }

    fn bottom(self) -> i32 {
        self.y.saturating_add(self.height)
    }

    fn expanded(self, padding_x: i32, padding_y: i32) -> Self {
        Self {
            x: self.x.saturating_sub(padding_x),
            y: self.y.saturating_sub(padding_y),
            width: self.width.saturating_add(padding_x.saturating_mul(2)),
            height: self.height.saturating_add(padding_y.saturating_mul(2)),
        }
    }

    fn intersects(self, other: Self) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }
}

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

#[cfg(desktop)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IslandScreenshotShortcutAction {
    Screenshot,
    Pin,
    Delay,
}

#[cfg(desktop)]
impl IslandScreenshotShortcutAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Screenshot => "screenshot",
            Self::Pin => "pin",
            Self::Delay => "delay",
        }
    }
}

#[cfg(desktop)]
#[derive(Debug, Clone, Copy)]
struct RegisteredIslandScreenshotShortcut {
    action: IslandScreenshotShortcutAction,
    shortcut: tauri_plugin_global_shortcut::Shortcut,
}

#[cfg(desktop)]
static ISLAND_SCREENSHOT_SHORTCUTS: OnceLock<StdMutex<Vec<RegisteredIslandScreenshotShortcut>>> =
    OnceLock::new();

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
                    .with_vision_support(vision_supported)
                    .with_tool_call_support(tool_calls_supported)
                    .with_json_mode_supported(json_mode_supported);
            if let Some(base_url) = &connection.base_url {
                client = client.with_base_url(crate::config::normalize_base_url(base_url));
            }
            Ok(Box::new(client))
        }
        "anthropic" => {
            let mut client =
                AnthropicClient::new(connection.api_key.clone(), connection.model.clone())
                    .with_vision_support(vision_supported)
                    .with_tool_call_support(tool_calls_supported);
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
    let local_db = LocalDb::open_default().expect("Failed to initialize Aura local database");
    local_db
        .mark_interrupted_agent_runs()
        .expect("Failed to reconcile interrupted Aura agent runs");

    // Patch 17 / #18: load hook config from ~/.aura/hooks.toml.
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

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());

    #[cfg(desktop)]
    let builder = builder.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, shortcut, event| {
                use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

                if event.state() != ShortcutState::Pressed {
                    return;
                }
                let toggle_shortcut =
                    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
                if shortcut == &toggle_shortcut {
                    toggle_float_window(app);
                    return;
                }
                if let Some(action) = find_island_screenshot_shortcut_action(shortcut) {
                    trigger_island_screenshot_shortcut(app, action);
                }
            })
            .build(),
    );

    builder
        .manage(state)
        .setup(|app| {
            commands::cleanup_expired_island_temp_files();
            setup_tray(app);
            setup_global_shortcut(app);
            sync_island_screenshot_shortcuts(&app.handle().clone());
            configure_float_window(&app.handle().clone());
            if std::env::var("AURA_SMOKE_RUN_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .is_some()
            {
                show_main_window(&app.handle().clone());
            }
            if std::env::var("AURA_SMOKE_SHOW_FLOAT").ok().as_deref() == Some("1") {
                if let Some(window) = app.get_webview_window("float") {
                    window.show()?;
                }
            }
            Ok(())
        })
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
            commands::get_backend_status,
            commands::write_settings_smoke_proof,
            commands::write_settings_persistence_smoke_proof,
            commands::write_settings_domain_smoke_proof,
            commands::write_agent_workbench_smoke_proof,
            commands::island_get_settings,
            commands::island_save_settings,
            commands::island_show_main_window,
            commands::island_get_window_context,
            commands::island_read_clipboard,
            commands::island_capture_screenshot,
            commands::island_sample_screen_pixel,
            commands::island_run_ocr,
            commands::island_check_shortcut_conflicts,
            commands::island_check_save_path_permission,
            commands::island_save_context_export,
            commands::island_get_media_status,
            commands::island_control_media,
            commands::island_get_system_status,
            commands::island_write_keyboard_smoke_proof,
            commands::island_log_context_sent,
            commands::island_log_context_imported,
            commands::island_cleanup_temp_file,
            commands::island_read_temp_image,
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

fn setup_tray(app: &mut tauri::App) {
    #[cfg(desktop)]
    {
        use tauri::{
            menu::{Menu, MenuItem},
            tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
        };

        let show_main = match MenuItem::with_id(app, "show-main", "打开 Aura", true, None::<&str>)
        {
            Ok(item) => item,
            Err(error) => {
                eprintln!("Aura tray menu item failed: {error}");
                return;
            }
        };
        let toggle_float = match MenuItem::with_id(
            app,
            "toggle-float",
            "显示/隐藏 Agent 浮层",
            true,
            None::<&str>,
        ) {
            Ok(item) => item,
            Err(error) => {
                eprintln!("Aura tray menu item failed: {error}");
                return;
            }
        };
        let privacy_pause =
            match MenuItem::with_id(app, "privacy-pause", "暂停系统感知", true, None::<&str>)
            {
                Ok(item) => item,
                Err(error) => {
                    eprintln!("Aura tray menu item failed: {error}");
                    return;
                }
            };
        let privacy_resume =
            match MenuItem::with_id(app, "privacy-resume", "恢复系统感知", true, None::<&str>)
            {
                Ok(item) => item,
                Err(error) => {
                    eprintln!("Aura tray menu item failed: {error}");
                    return;
                }
            };
        let quit = match MenuItem::with_id(app, "quit", "退出 Aura", true, None::<&str>) {
            Ok(item) => item,
            Err(error) => {
                eprintln!("Aura tray menu item failed: {error}");
                return;
            }
        };
        let menu = match Menu::with_items(
            app,
            &[
                &show_main,
                &toggle_float,
                &privacy_pause,
                &privacy_resume,
                &quit,
            ],
        ) {
            Ok(menu) => menu,
            Err(error) => {
                eprintln!("Aura tray menu failed: {error}");
                return;
            }
        };

        let mut builder = TrayIconBuilder::new()
            .menu(&menu)
            .tooltip("Aura")
            .show_menu_on_left_click(true)
            .on_menu_event(|app, event| match event.id.as_ref() {
                "show-main" => show_main_window(app),
                "toggle-float" => toggle_float_window(app),
                "privacy-pause" => set_island_privacy_from_app(app, true),
                "privacy-resume" => set_island_privacy_from_app(app, false),
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_main_window(tray.app_handle());
                }
            });

        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
        if let Err(error) = builder.build(app) {
            eprintln!("Aura tray setup failed: {error}");
        }
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
    }
}

fn setup_global_shortcut(app: &mut tauri::App) {
    #[cfg(desktop)]
    {
        use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

        if std::env::var("AURA_SMOKE_SKIP_GLOBAL_SHORTCUT")
            .ok()
            .as_deref()
            == Some("1")
        {
            eprintln!("Aura global shortcut registration skipped by smoke env");
            return;
        }

        let toggle_shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
        if let Err(error) = app.global_shortcut().register(toggle_shortcut) {
            eprintln!("Aura global shortcut registration failed: {error}");
        } else {
            eprintln!("Aura global shortcut registered: Ctrl+Alt+Space");
        }
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
    }
}

#[cfg(desktop)]
fn island_screenshot_shortcuts() -> &'static StdMutex<Vec<RegisteredIslandScreenshotShortcut>> {
    ISLAND_SCREENSHOT_SHORTCUTS.get_or_init(|| StdMutex::new(Vec::new()))
}

#[cfg(desktop)]
fn configured_island_screenshot_shortcuts(
    settings: &commands::IslandSettingsPayload,
) -> Vec<(IslandScreenshotShortcutAction, &'static str, String)> {
    let mut shortcuts = vec![
        (
            IslandScreenshotShortcutAction::Screenshot,
            "主截图",
            settings.screenshot.main_shortcut.clone(),
        ),
        (
            IslandScreenshotShortcutAction::Pin,
            "贴图",
            settings.screenshot.pin_shortcut.clone(),
        ),
        (
            IslandScreenshotShortcutAction::Delay,
            "延时截图",
            settings.screenshot.delay_shortcut.clone(),
        ),
    ];
    apply_smoke_screenshot_shortcut_overrides(&mut shortcuts);
    shortcuts
}

#[cfg(desktop)]
fn apply_smoke_screenshot_shortcut_overrides(
    shortcuts: &mut [(IslandScreenshotShortcutAction, &'static str, String)],
) {
    if std::env::var("AURA_SMOKE_RUN_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_none()
    {
        return;
    }

    for (action, _, accelerator) in shortcuts.iter_mut() {
        let env_name = match action {
            IslandScreenshotShortcutAction::Screenshot => "AURA_SMOKE_SCREENSHOT_MAIN_SHORTCUT",
            IslandScreenshotShortcutAction::Pin => "AURA_SMOKE_SCREENSHOT_PIN_SHORTCUT",
            IslandScreenshotShortcutAction::Delay => "AURA_SMOKE_SCREENSHOT_DELAY_SHORTCUT",
        };
        if let Ok(value) = std::env::var(env_name) {
            let value = value.trim();
            if !value.is_empty() {
                *accelerator = value.to_string();
            }
        }
    }
}

#[cfg(desktop)]
pub(crate) fn sync_island_screenshot_shortcuts<R: Runtime>(app: &tauri::AppHandle<R>) {
    use std::collections::HashSet;
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

    if std::env::var("AURA_SMOKE_SKIP_GLOBAL_SHORTCUT")
        .ok()
        .as_deref()
        == Some("1")
    {
        eprintln!("Aura screenshot shortcut registration skipped by smoke env");
        return;
    }

    let shortcut_manager = app.global_shortcut();
    let mut registry = match island_screenshot_shortcuts().lock() {
        Ok(registry) => registry,
        Err(_) => {
            eprintln!("Aura screenshot shortcut registry unavailable");
            return;
        }
    };

    for registered in registry.drain(..) {
        if let Err(error) = shortcut_manager.unregister(registered.shortcut) {
            eprintln!(
                "Aura screenshot shortcut unregister failed ({}): {error}",
                registered.action.as_str()
            );
        }
    }

    let state = app.state::<AppState>();
    let settings = match commands::load_island_settings_from_db(&state.local_db) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("Aura screenshot shortcut settings load failed: {error}");
            return;
        }
    };
    let mut seen = HashSet::new();
    for (action, label, accelerator) in configured_island_screenshot_shortcuts(&settings) {
        let accelerator = accelerator.trim().to_string();
        if accelerator.is_empty() {
            eprintln!("Aura screenshot shortcut skipped ({label}): empty accelerator");
            continue;
        }
        let dedupe_key = accelerator.to_ascii_lowercase().replace(' ', "");
        if !seen.insert(dedupe_key) {
            eprintln!(
                "Aura screenshot shortcut skipped ({label}): duplicate accelerator {accelerator}"
            );
            continue;
        }
        let shortcut = match Shortcut::from_str(&accelerator) {
            Ok(shortcut) => shortcut,
            Err(error) => {
                eprintln!(
                    "Aura screenshot shortcut skipped ({label} {accelerator}): invalid accelerator: {error}"
                );
                continue;
            }
        };
        if shortcut_manager.is_registered(shortcut) {
            eprintln!(
                "Aura screenshot shortcut skipped ({label} {accelerator}): already registered by Aura"
            );
            continue;
        }
        match shortcut_manager.register(shortcut) {
            Ok(_) => {
                registry.push(RegisteredIslandScreenshotShortcut { action, shortcut });
                eprintln!("Aura screenshot shortcut registered: {label} {accelerator}");
            }
            Err(error) => {
                eprintln!(
                    "Aura screenshot shortcut registration failed ({label} {accelerator}): {error}"
                );
            }
        }
    }
}

#[cfg(not(desktop))]
pub(crate) fn sync_island_screenshot_shortcuts<R: Runtime>(_app: &tauri::AppHandle<R>) {}

#[cfg(desktop)]
fn find_island_screenshot_shortcut_action(
    shortcut: &tauri_plugin_global_shortcut::Shortcut,
) -> Option<IslandScreenshotShortcutAction> {
    island_screenshot_shortcuts()
        .lock()
        .ok()
        .and_then(|registry| {
            registry
                .iter()
                .find(|registered| &registered.shortcut == shortcut)
                .map(|registered| registered.action)
        })
}

#[cfg(desktop)]
pub(crate) fn is_registered_island_screenshot_shortcut(
    shortcut: &tauri_plugin_global_shortcut::Shortcut,
) -> bool {
    island_screenshot_shortcuts()
        .lock()
        .map(|registry| {
            registry
                .iter()
                .any(|registered| &registered.shortcut == shortcut)
        })
        .unwrap_or(false)
}

#[cfg(desktop)]
fn trigger_island_screenshot_shortcut<R: Runtime>(
    app: &tauri::AppHandle<R>,
    action: IslandScreenshotShortcutAction,
) {
    set_island_manual_hidden_from_app(app, false);
    configure_float_window(app);
    let mut float_window_found = false;
    let mut float_show_ok = false;
    if let Some(window) = app.get_webview_window("float") {
        float_window_found = true;
        float_show_ok = window.show().is_ok();
    }
    std::thread::sleep(std::time::Duration::from_millis(180));
    let payload = json!({
        "source": "global_shortcut",
        "action": action.as_str(),
        "triggeredAt": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0),
    });
    let mut dispatch_method = "emit_to";
    let mut dispatch_ok = false;
    let mut dispatch_error = None;
    if let Some(window) = app.get_webview_window("float") {
        let payload_json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
        let script = format!(
            "window.dispatchEvent(new CustomEvent('aura:island-shortcut', {{ detail: {} }}));",
            payload_json
        );
        dispatch_method = "webview_eval";
        match window.eval(&script) {
            Ok(_) => dispatch_ok = true,
            Err(error) => dispatch_error = Some(error.to_string()),
        }
    }
    if !dispatch_ok {
        let emit_result = app.emit_to("float", "aura-island-shortcut", payload);
        dispatch_method = "emit_to";
        dispatch_ok = emit_result.is_ok();
        if let Err(error) = emit_result {
            dispatch_error = Some(error.to_string());
        }
    }
    write_island_shortcut_smoke_proof(
        action,
        "global_shortcut_event",
        float_window_found,
        float_show_ok,
        dispatch_ok,
        dispatch_method,
        dispatch_error.clone(),
    );
    if let Some(error) = dispatch_error {
        eprintln!(
            "Aura screenshot shortcut event dispatch failed ({} via {}): {error}",
            action.as_str(),
            dispatch_method
        );
    } else {
        eprintln!(
            "Aura screenshot shortcut event dispatched ({} via {})",
            action.as_str(),
            dispatch_method
        );
    }
}

#[cfg(desktop)]
fn write_island_shortcut_smoke_proof(
    action: IslandScreenshotShortcutAction,
    stage: &str,
    float_window_found: bool,
    float_show_ok: bool,
    emit_ok: bool,
    dispatch_method: &str,
    error: Option<String>,
) {
    if std::env::var("AURA_SMOKE_ENABLE_ISLAND_SHORTCUT_PROOF")
        .ok()
        .as_deref()
        != Some("1")
        && std::env::var("AURA_SMOKE_ENABLE_ISLAND_CAPABILITIES")
            .ok()
            .as_deref()
            != Some("1")
    {
        return;
    }

    let smoke_run_id = std::env::var("AURA_SMOKE_RUN_ID").unwrap_or_default();
    if smoke_run_id.trim().is_empty() {
        return;
    }
    let captured_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let proof = json!({
        "ok": emit_ok,
        "kind": "island_shortcut_smoke_proof",
        "smokeRunId": smoke_run_id,
        "source": "global_shortcut",
        "action": action.as_str(),
        "stage": stage,
        "floatWindowFound": float_window_found,
        "floatShowOk": float_show_ok,
        "emitOk": emit_ok,
        "dispatchMethod": dispatch_method,
        "error": error,
        "capturedAt": captured_at,
    });
    let path = std::env::temp_dir().join(format!(
        "aura-island-shortcut-smoke-{}-{}.json",
        proof
            .get("smokeRunId")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("unknown"),
        uuid::Uuid::new_v4()
    ));
    if let Ok(bytes) = serde_json::to_vec(&proof) {
        let _ = std::fs::write(path, bytes);
    }
}

fn show_main_window<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title("Aura");
        let _ = window.set_min_size(Some(LogicalSize::new(1100.0, 700.0)));
        let _ = window.set_size(LogicalSize::new(1360.0, 860.0));
        let _ = window.center();
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_float_window<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("float") {
        let is_visible = window.is_visible().unwrap_or(false);
        if is_visible {
            let _ = window.hide();
            set_island_manual_hidden_from_app(app, true);
        } else {
            set_island_manual_hidden_from_app(app, false);
            configure_float_window(app);
            let _ = window.show();
        }
    }
}

fn configure_float_window<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("float") {
        let _ = window.set_always_on_top(true);
        let _ = window.set_skip_taskbar(true);
        let _ = window.set_focusable(false);
        position_float_window(app, &window);
    }
}

fn position_float_window<R: Runtime>(app: &tauri::AppHandle<R>, window: &tauri::WebviewWindow<R>) {
    let Ok(Some(monitor)) = app.primary_monitor() else {
        return;
    };
    let work_area = monitor.work_area();
    let Ok(size) = window.outer_size() else {
        return;
    };
    let work_rect = DesktopRect {
        x: work_area.position.x,
        y: work_area.position.y,
        width: work_area.size.width as i32,
        height: work_area.size.height as i32,
    };
    let margin = (12.0 * monitor.scale_factor()).round().clamp(8.0, 32.0) as i32;
    let avoid_rect = foreground_text_input_avoid_rect()
        .map(|rect| rect.expanded(size.width as i32 / 2, (size.height as i32).max(96)))
        .or_else(|| foreground_window_fallback_avoid_rect(work_rect));
    let target = choose_float_position(
        work_rect,
        size.width as i32,
        size.height as i32,
        margin,
        avoid_rect,
    );
    let _ = window.set_position(PhysicalPosition::new(target.x, target.y));
}

fn choose_float_position(
    work_area: DesktopRect,
    width: i32,
    height: i32,
    margin: i32,
    avoid_rect: Option<DesktopRect>,
) -> DesktopRect {
    let max_x = work_area
        .right()
        .saturating_sub(width)
        .saturating_sub(margin);
    let max_y = work_area
        .bottom()
        .saturating_sub(height)
        .saturating_sub(margin);
    let min_x = work_area.x.saturating_add(margin);
    let min_y = work_area.y.saturating_add(margin);
    let right_x = max_x.max(min_x);
    let bottom_y = max_y.max(min_y);
    let centered_x = work_area.x.saturating_add((work_area.width - width) / 2);
    let center_x = centered_x.clamp(min_x, right_x);

    let candidates = [
        DesktopRect {
            x: center_x,
            y: min_y.clamp(min_y, bottom_y),
            width,
            height,
        },
        DesktopRect {
            x: right_x,
            y: min_y.clamp(min_y, bottom_y),
            width,
            height,
        },
        DesktopRect {
            x: min_x,
            y: min_y.clamp(min_y, bottom_y),
            width,
            height,
        },
        DesktopRect {
            x: center_x,
            y: bottom_y,
            width,
            height,
        },
        DesktopRect {
            x: right_x,
            y: bottom_y,
            width,
            height,
        },
        DesktopRect {
            x: min_x,
            y: bottom_y,
            width,
            height,
        },
    ];

    if let Some(avoid) = avoid_rect {
        if let Some(candidate) = candidates
            .iter()
            .copied()
            .find(|candidate| !candidate.intersects(avoid))
        {
            return candidate;
        }
    }

    candidates[0]
}

#[cfg(test)]
fn fallback_avoid_rect_for_foreground_window(
    work_area: DesktopRect,
    foreground: DesktopRect,
) -> Option<DesktopRect> {
    fallback_avoid_rect_from_foreground_candidates(work_area, &[foreground])
}

fn fallback_avoid_rect_from_foreground_candidates(
    work_area: DesktopRect,
    candidates: &[DesktopRect],
) -> Option<DesktopRect> {
    candidates.iter().copied().find_map(|foreground| {
        fallback_avoid_rect_for_single_foreground_window(work_area, foreground)
    })
}

fn fallback_avoid_rect_for_single_foreground_window(
    work_area: DesktopRect,
    foreground: DesktopRect,
) -> Option<DesktopRect> {
    if foreground.width < 120 || foreground.height < 80 {
        return None;
    }
    let work_area_size = (work_area.width as i64).saturating_mul(work_area.height as i64);
    let foreground_size = (foreground.width as i64).saturating_mul(foreground.height as i64);
    if work_area_size <= 0
        || foreground_size.saturating_mul(100) >= work_area_size.saturating_mul(75)
    {
        return None;
    }

    let top_band_height = foreground.height.clamp(80, 180);
    Some(DesktopRect {
        x: foreground.x,
        y: foreground.y,
        width: foreground.width,
        height: top_band_height,
    })
}

#[cfg(windows)]
fn foreground_text_input_avoid_rect() -> Option<DesktopRect> {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }

        let thread_id = GetWindowThreadProcessId(hwnd, None);
        if thread_id == 0 {
            return None;
        }

        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(thread_id, &mut info).is_err() || info.hwndCaret.is_invalid() {
            return None;
        }

        let RECT {
            left,
            top,
            right,
            bottom,
        } = info.rcCaret;
        if right <= left || bottom <= top {
            return None;
        }

        let mut top_left = POINT { x: left, y: top };
        let mut bottom_right = POINT {
            x: right,
            y: bottom,
        };
        if !ClientToScreen(info.hwndCaret, &mut top_left).as_bool()
            || !ClientToScreen(info.hwndCaret, &mut bottom_right).as_bool()
        {
            return None;
        }

        Some(DesktopRect {
            x: top_left.x.min(bottom_right.x),
            y: top_left.y.min(bottom_right.y),
            width: (top_left.x - bottom_right.x).abs().max(1),
            height: (top_left.y - bottom_right.y).abs().max(1),
        })
    }
}

#[cfg(not(windows))]
fn foreground_text_input_avoid_rect() -> Option<DesktopRect> {
    None
}

#[cfg(windows)]
fn foreground_window_fallback_avoid_rect(work_area: DesktopRect) -> Option<DesktopRect> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetForegroundWindow, GetWindowRect, GA_ROOTOWNER,
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }

        let root_owner = GetAncestor(hwnd, GA_ROOTOWNER);
        let mut candidates = Vec::new();
        for candidate in [root_owner, hwnd] {
            if candidate.is_invalid() {
                continue;
            }
            let mut rect = RECT::default();
            if GetWindowRect(candidate, &mut rect).is_ok() {
                candidates.push(DesktopRect {
                    x: rect.left,
                    y: rect.top,
                    width: rect.right.saturating_sub(rect.left),
                    height: rect.bottom.saturating_sub(rect.top),
                });
            }
        }
        fallback_avoid_rect_from_foreground_candidates(work_area, &candidates)
    }
}

#[cfg(not(windows))]
fn foreground_window_fallback_avoid_rect(_work_area: DesktopRect) -> Option<DesktopRect> {
    None
}

fn set_island_manual_hidden_from_app<R: Runtime>(app: &tauri::AppHandle<R>, manual_hidden: bool) {
    let state = app.state::<AppState>();
    match commands::persist_island_manual_hidden(&state.local_db, manual_hidden) {
        Ok(settings) => {
            let _ = app.emit("island-settings-changed", settings);
        }
        Err(error) => eprintln!("Aura island manual hidden update failed: {error}"),
    }
}

fn set_island_privacy_from_app<R: Runtime>(app: &tauri::AppHandle<R>, privacy_paused: bool) {
    let state = app.state::<AppState>();
    match commands::persist_island_privacy_paused(&state.local_db, privacy_paused) {
        Ok(settings) => {
            let _ = app.emit("island-settings-changed", settings);
        }
        Err(error) => eprintln!("Aura island privacy update failed: {error}"),
    }
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
        LocalDb::open(std::env::temp_dir().join(format!("aura_tool_registry_{unique}.db"))).unwrap()
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

    #[test]
    fn float_position_defaults_to_top_center_without_avoid_rect() {
        let work_area = DesktopRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1032,
        };

        let position = choose_float_position(work_area, 96, 89, 12, None);

        assert_eq!(
            position,
            DesktopRect {
                x: 912,
                y: 12,
                width: 96,
                height: 89
            }
        );
    }

    #[test]
    fn float_position_avoids_top_center_text_input() {
        let work_area = DesktopRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1032,
        };
        let avoid = DesktopRect {
            x: 860,
            y: 0,
            width: 240,
            height: 140,
        };

        let position = choose_float_position(work_area, 96, 89, 12, Some(avoid));

        assert_eq!(position.x, 1812);
        assert_eq!(position.y, 12);
        assert!(!position.intersects(avoid));
    }

    #[test]
    fn foreground_window_fallback_uses_medium_window_top_band() {
        let work_area = DesktopRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1032,
        };
        let foreground = DesktopRect {
            x: 912,
            y: 12,
            width: 720,
            height: 360,
        };

        let avoid = fallback_avoid_rect_for_foreground_window(work_area, foreground)
            .expect("medium foreground window should produce a fallback avoid rect");
        let position = choose_float_position(work_area, 96, 89, 12, Some(avoid));

        assert_eq!(
            avoid,
            DesktopRect {
                x: 912,
                y: 12,
                width: 720,
                height: 180
            }
        );
        assert_eq!(position.x, 1812);
        assert_eq!(position.y, 12);
        assert!(!position.intersects(avoid));
    }

    #[test]
    fn foreground_window_fallback_ignores_near_fullscreen_windows() {
        let work_area = DesktopRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1032,
        };
        let foreground = DesktopRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1032,
        };

        assert_eq!(
            fallback_avoid_rect_for_foreground_window(work_area, foreground),
            None
        );
    }

    #[test]
    fn foreground_window_fallback_prefers_root_owner_over_browser_child_window() {
        let work_area = DesktopRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1032,
        };
        let root_owner = DesktopRect {
            x: 912,
            y: 12,
            width: 720,
            height: 360,
        };
        let browser_child = DesktopRect {
            x: 1021,
            y: 88,
            width: 502,
            height: 293,
        };

        let avoid =
            fallback_avoid_rect_from_foreground_candidates(work_area, &[root_owner, browser_child])
                .expect("browser root owner should produce the fallback avoid rect");
        let position = choose_float_position(work_area, 96, 89, 12, Some(avoid));

        assert_eq!(
            avoid,
            DesktopRect {
                x: 912,
                y: 12,
                width: 720,
                height: 180
            }
        );
        assert_eq!(position.x, 1812);
        assert_eq!(position.y, 12);
        assert!(!position.intersects(avoid));
    }

    #[test]
    fn float_position_falls_back_when_every_candidate_intersects() {
        let work_area = DesktopRect {
            x: 0,
            y: 0,
            width: 300,
            height: 240,
        };
        let avoid = DesktopRect {
            x: 0,
            y: 0,
            width: 300,
            height: 240,
        };

        let position = choose_float_position(work_area, 96, 89, 12, Some(avoid));

        assert_eq!(
            position,
            DesktopRect {
                x: 102,
                y: 12,
                width: 96,
                height: 89
            }
        );
    }
}
