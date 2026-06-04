import { invoke } from '@tauri-apps/api/core';
import type { IslandSettings } from './island-types';

export type TauriCmd =
  | 'agent_chat'
  | 'agent_chat_v2'
  | 'agent_subagent_chat'
  | 'summarize_session'
  | 'get_context_usage'
  | 'cancel_agent_chat'
  | 'pause_agent_chat'
  | 'resume_agent_chat'
  | 'get_agent_skill_metadata'
  | 'get_agent_subagent_metadata'
  | 'get_agent_skill_input'
  | 'get_model_usage_summary'
  | 'get_agent_runs'
  | 'get_agent_run_steps'
  | 'get_agent_run_timeline'
  | 'get_browser_agent_steps'
  | 'create_agent_graph_run'
  | 'get_agent_graph_snapshot'
  | 'pause_agent_graph_run'
  | 'resume_agent_graph_run'
  | 'create_team_run'
  | 'get_team_run_snapshot'
  | 'append_team_message'
  | 'create_handoff_request'
  | 'resolve_handoff_request'
  | 'apply_team_termination'
  | 'schedule_team_execution_plan'
  | 'pause_team_execution'
  | 'resume_team_execution'
  | 'add_knowledge_item'
  | 'search_knowledge'
  | 'delete_knowledge_item'
  | 'ingest_connector_knowledge_items'
  | 'write_agent_memory_event'
  | 'create_workspace_lifecycle'
  | 'get_workspace_lifecycle_snapshot'
  | 'validate_workspace_cwd'
  | 'run_workspace_setup_script'
  | 'validate_workspace_command_binding'
  | 'install_workspace_git_hook'
  | 'export_agent_run_trajectory'
  | 'replay_agent_run_trajectory'
  | 'evaluate_agent_run_trajectory'
  | 'create_external_agent_task'
  | 'get_external_agent_task_mapping'
  | 'get_protocol_compatibility_matrix'
  | 'update_external_agent_task_lifecycle'
  | 'cancel_external_agent_task'
  | 'append_external_agent_task_stream_event'
  | 'list_external_agent_task_stream_events'
  | 'evaluate_plugin_quality_gate'
  | 'get_plugin_eval_registry_entry'
  | 'get_skill_version_registry'
  | 'get_team_preset_permission_report'
  | 'list_plugin_packages'
  | 'set_plugin_package_enabled'
  | 'get_plugin_capability_events'
  | 'inspect_code_intelligence_report'
  | 'prepare_lsp_session_plan'
  | 'estimate_model_cost'
  | 'estimate_model_text_cost'
  | 'record_model_quality_event'
  | 'get_model_quality_events'
  | 'get_model_route_decisions'
  | 'explain_route_economics'
  | 'get_agent_graph_node_traces'
  | 'enqueue_agent_graph_run'
  | 'abort_queued_agent_graph_run'
  | 'list_agent_graph_queue'
  | 'set_agent_graph_queue_paused'
  | 'get_agent_graph_queue_control'
  | 'get_agent_run_diff'
  | 'get_agent_run_terminal'
  | 'get_agent_run_audit'
  | 'get_agent_run_progress'
  | 'get_agent_status_semantic'
  | 'get_agent_permission_decisions'
  | 'get_agent_eval_suites'
  | 'score_agent_eval_suite'
  | 'run_agent_eval_suite_verifiers'
  | 'resolve_permission_confirmation'
  | 'classify_user_intent'
  | 'get_project_snapshot'
  | 'get_agent_tool_audit_events'
  | 'retry_agent_run'
  | 'start_feishu_callback_server'
  | 'stop_feishu_callback_server'
  | 'get_feishu_callback_status'
  | 'set_feishu_public_url'
  | 'get_feishu_setup_links'
  | 'start_feishu_public_tunnel'
  | 'stop_feishu_public_tunnel'
  | 'get_feishu_tunnel_status'
  | 'get_feishu_received_events'
  | 'ingest_feishu_event_payload'
  | 'get_config'
  | 'save_config'
  | 'get_backend_status'
  | 'write_settings_smoke_proof'
  | 'write_settings_persistence_smoke_proof'
  | 'write_settings_domain_smoke_proof'
  | 'write_agent_workbench_smoke_proof'
  | 'island_get_settings'
  | 'island_save_settings'
  | 'island_show_main_window'
  | 'island_get_window_context'
  | 'island_read_clipboard'
  | 'island_capture_screenshot'
  | 'island_sample_screen_pixel'
  | 'island_run_ocr'
  | 'island_check_shortcut_conflicts'
  | 'island_check_save_path_permission'
  | 'island_save_context_export'
  | 'island_get_media_status'
  | 'island_control_media'
  | 'island_get_system_status'
  | 'island_write_keyboard_smoke_proof'
  | 'island_log_context_sent'
  | 'island_log_context_imported'
  | 'island_cleanup_temp_file'
  | 'island_read_temp_image'
  | 'check_model_settings'
  | 'list_models'
  | 'search_web'
  | 'open_external_web_search'
  | 'fetch_web_page'
  | 'get_github_trending'
  | 'init_local_db'
  | 'get_sessions'
  | 'list_projects'
  | 'upsert_project'
  | 'create_project_folder'
  | 'rename_project'
  | 'set_project_pinned'
  | 'archive_project'
  | 'delete_project'
  | 'open_project_in_explorer'
  | 'generate_history_report'
  | 'get_archived_sessions'
  | 'search_sessions'
  | 'create_session'
  | 'rename_session'
  | 'delete_session'
  | 'archive_session'
  | 'restore_session'
  | 'set_session_pinned'
  | 'get_messages'
  | 'save_message'
  | 'clear_session_context'
  | 'get_plan_tasks'
  | 'create_plan_task'
  | 'update_plan_task'
  | 'update_plan_task_status'
  | 'archive_plan_task'
  | 'set_active_plan_task'
  | 'waive_plan_task'
  | 'get_plan_change_events'
  | 'scan_plan_task_run_integrity'
  | 'repair_plan_task_run_integrity'
  | 'git_status'
  | 'git_diff'
  | 'git_log'
  | 'git_show'
  | 'git_stage'
  | 'git_commit'
  | 'git_create_branch'
  | 'git_push'
  | 'list_provider_capabilities'
  | 'resolve_provider_capabilities'
  | 'probe_provider_capabilities'
  | 'reset_provider_capabilities'
  | 'list_capability_audit'
  | 'override_provider_capabilities'
  | 'init_agent_rules'
  | 'read_global_agent_rules'
  | 'save_global_agent_rules'
  | 'get_code_review_command_rules'
  | 'get_mcp_servers'
  | 'save_mcp_server'
  | 'delete_mcp_server'
  | 'set_mcp_server_trust'
  | 'test_mcp_server'
  | 'invoke_mcp_tool'
  | 'get_mcp_audit_events'
  | 'get_memories'
  | 'add_memory'
  | 'update_memory'
  | 'delete_memory'
  | 'clear_memories'
  | 'get_profile'
  | 'save_profile'
  | 'start_personality_test'
  | 'get_personality_questions'
  | 'get_personality_progress'
  | 'save_personality_progress'
  | 'complete_personality_test'
  | 'prepare_file_write'
  | 'confirm_file_write'
  | 'reject_file_write'
  | 'run_approved_command'
  | 'reject_pending_command'
  | 'get_app_state'
  | 'set_app_state'
  | 'get_personality_onboarding_state'
  | 'save_personality_onboarding_state'
  | 'export_local_data'
  | 'reset_local_data'
  | 'get_local_db_health'
  | 'log_activity_event'
  | 'get_recent_activity_events'
  | 'get_browser_audit_events'
  | 'install_plugin_package'
  | 'invoke_plugin_capability'
  | 'get_artifacts';

export class AuraInvokeError extends Error {
  command: TauriCmd;
  userMessage: string;

  constructor(command: TauriCmd, cause: unknown) {
    const raw = cause instanceof Error ? cause.message : String(cause ?? 'Unknown error');
    super(raw);
    this.name = 'AuraInvokeError';
    this.command = command;
    this.userMessage = toReadableUserMessage(command, raw);
  }
}

export async function invokeCmd<T>(cmd: TauriCmd, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
    throw new AuraInvokeError(cmd, 'Aura desktop backend is not connected in browser preview.');
  }

  try {
    return await invoke<T>(cmd, args);
  } catch (error) {
    throw new AuraInvokeError(cmd, error);
  }
}

export function isTauriRuntime() {
  if (typeof window === 'undefined') return false;
  const tauriInternals = (window as Window & { __TAURI_INTERNALS__?: { invoke?: unknown } }).__TAURI_INTERNALS__;
  return typeof tauriInternals?.invoke === 'function';
}

export function getErrorMessage(error: unknown) {
  if (error instanceof AuraInvokeError) return error.userMessage;
  if (error instanceof Error) return error.message;
  return String(error ?? 'Unknown error');
}



function toReadableUserMessage(cmd: TauriCmd, raw: string) {
  const lower = raw.toLowerCase();
  const cn = (text: string) => text;

  if (lower.includes('目标路径属于系统') || lower.includes('sensitive application')) {
    return cn('目标路径属于系统、密钥或敏感应用目录，Aura 不会写入。');
  }
  if (lower.includes('只允许准备写入') || lower.includes('md、txt、json、csv、log')) {
    return cn('Aura 第一版只允许写入 md、txt、json、csv、log 文本文件。');
  }
  if (lower.includes('desktop backend is not connected')) {
    return cn('当前运行环境暂不可用，请在 Aura 客户端中测试。');
  }
  if (lower.includes("reading 'invoke'") || lower.includes('reading "invoke"') || lower.includes('__tauri_internals__') || lower.includes('transformcallback')) {
    return cn('当前运行环境暂不可用，请在 Aura 客户端中测试。');
  }
  if (lower.includes('command') && lower.includes('not found')) {
    return cn('当前 Aura 桌面后端缺少这个本地命令。请完全退出并重新启动 Aura 后再试。');
  }
  if (lower.includes('desktop backend') || lower.includes('browser preview')) {
    return cn('当前运行环境暂不可用，请在 Aura 客户端中测试。');
  }
  if (lower.includes('missing api key') || lower.includes('api key')) {
    return cn('\u6a21\u578b\u5bc6\u94a5\u5c1a\u672a\u914d\u7f6e\uff0c\u8bf7\u5148\u5230\u8bbe\u7f6e\u9875\u4fdd\u5b58\u5bc6\u94a5\u3002');
  }
  if (lower.includes('unsupported provider')) {
    return cn('当前模型类型不受支持，请选择 OpenAI 兼容、Claude 兼容、Ollama 或 LM Studio。');
  }
  if (lower.includes('cancelled')) return cn('\u672c\u6b21\u5bf9\u8bdd\u5df2\u53d6\u6d88\u3002');
  if (lower.includes('timeout') || lower.includes('timed out')) return cn('\u8bf7\u6c42\u8d85\u65f6\uff0c\u8bf7\u68c0\u67e5\u7f51\u7edc\u6216\u672c\u5730\u6a21\u578b\u670d\u52a1\u3002');
  if (lower.includes('connection refused') || lower.includes('connect error')) return cn('\u65e0\u6cd5\u8fde\u63a5\u6a21\u578b\u670d\u52a1\uff0c\u8bf7\u68c0\u67e5 URL \u6216\u672c\u5730\u6a21\u578b\u670d\u52a1\u662f\u5426\u542f\u52a8\u3002');
  if (lower.includes('401') || lower.includes('unauthorized')) return cn('认证失败，请检查模型密钥。');
  if (lower.includes('404') && (cmd === 'agent_chat' || cmd === 'agent_chat_v2')) return cn('模型接口地址或模型名可能不正确，请检查 URL 和模型名。');
  if ((cmd === 'agent_chat' || cmd === 'agent_chat_v2') && lower.includes('openai-compatible') && lower.includes('failed with status')) {
    return cn(`模型请求失败：${raw.replace(/^LLM error:\s*/i, '')}`);
  }
  if ((cmd === 'agent_chat' || cmd === 'agent_chat_v2') && lower.includes('anthropic') && lower.includes('failed')) {
    return cn(`模型请求失败：${raw.replace(/^LLM error:\s*/i, '')}`);
  }
  if ((cmd === 'agent_chat' || cmd === 'agent_chat_v2') && lower.includes('llm error')) {
    return cn(`模型请求失败：${raw.replace(/^LLM error:\s*/i, '')}`);
  }
  if (lower.includes('429') || lower.includes('rate limit')) return cn('\u670d\u52a1\u89e6\u53d1\u9650\u6d41\uff0c\u8bf7\u7a0d\u540e\u518d\u8bd5\u3002');
  if (raw.includes('empty keyword') || raw.includes('Search keyword cannot be empty')) return cn('\u8bf7\u8f93\u5165\u641c\u7d22\u5173\u952e\u8bcd\u3002');

  switch (cmd) {
    case 'agent_chat':
    case 'agent_chat_v2':
      if (lower.includes('too many consecutive tool errors')) {
        return cn('工具连续失败，Aura 已停止本次任务。');
      }
      if (lower.includes('400') || lower.includes('bad request')) {
        return cn('模型接口拒绝了这次请求，请检查模型、URL 或附件格式。');
      }
      if (lower.includes('tool error')) {
        return cn('工具执行失败，Aura 已停止本次任务。');
      }
      return cn('Agent 暂时不可用，请稍后重试。');
    case 'save_config':
      return `${cn('\u914d\u7f6e\u4fdd\u5b58\u5931\u8d25\uff1a')}${raw}`;
    case 'get_config':
      return `${cn('\u914d\u7f6e\u8bfb\u53d6\u5931\u8d25\uff1a')}${raw}`;
    case 'check_model_settings':
    case 'list_models':
      return `${cn('模型连接检查失败：')}${raw}`;
    case 'search_web':
    case 'open_external_web_search':
    case 'fetch_web_page':
      return `${cn('网页/浏览器工具失败：')}${raw}`;
    case 'prepare_file_write':
    case 'confirm_file_write':
    case 'reject_file_write':
      return `${cn('文件写入确认失败：')}${raw}`;
    case 'run_approved_command':
      return `${cn('命令运行失败：')}${raw}`;
    case 'reject_pending_command':
      return `${cn('命令拒绝失败：')}${raw}`;
    default:
      return raw;
  }
}



export interface ChatHistoryItem {
  role: string;
  content: string;
}

export async function agentChat(message: string, history?: ChatHistoryItem[]): Promise<string> {
  return invokeCmd<string>('agent_chat', { message, history: history || [] });
}

export type AgentChatMode = 'chat' | 'plan' | 'code_review';

export interface AgentAttachmentPayload {
  id: string;
  name: string;
  mime: string;
  size: number;
  kind: 'image' | 'text' | 'file';
  dataUrl?: string;
  textPreview?: string;
  islandPackageId?: string;
}

export async function agentChatV2(
  sessionId: string,
  message: string,
  mode: AgentChatMode = 'chat',
  displayMessage?: string,
  attachments?: AgentAttachmentPayload[],
): Promise<string> {
  const args: Record<string, unknown> = { sessionId, message, mode };
  if (displayMessage !== undefined) args.displayMessage = displayMessage;
  if (attachments?.length) args.attachments = attachments;
  return invokeCmd<string>(
    'agent_chat_v2',
    args,
  );
}

export async function agentSubagentChat(
  sessionId: string,
  agentName: string,
  task: string,
  mode: AgentChatMode = 'chat',
  displayMessage?: string,
): Promise<string> {
  return invokeCmd<string>(
    'agent_subagent_chat',
    displayMessage
      ? { sessionId, agentName, task, mode, displayMessage }
      : { sessionId, agentName, task, mode },
  );
}

export async function summarizeSession(sessionId: string) {
  return invokeCmd<SessionSummary>('summarize_session', { sessionId });
}

export async function getContextUsage(
  sessionId: string,
  draftMessage = '',
  mode: AgentChatMode = 'chat',
  attachments: AgentAttachmentPayload[] = [],
) {
  return invokeCmd<ContextUsageSnapshot>('get_context_usage', {
    sessionId,
    draftMessage,
    mode,
    attachments,
  });
}

export interface CancelAgentChatResult {
  cancelledCount: number;
  sessionId?: string | null;
  scope: 'session' | 'all' | string;
}

export async function cancelAgentChat(sessionId?: string): Promise<CancelAgentChatResult> {
  return invokeCmd<CancelAgentChatResult>('cancel_agent_chat', sessionId ? { sessionId } : undefined);
}

export interface PauseAgentChatResult {
  runId?: string | null;
  /** 是否真的发生状态翻转(幂等:重复 pause/resume 不再翻转)。 */
  changed: boolean;
  status: 'paused' | 'running' | string;
}

export async function pauseAgentChat(sessionId?: string): Promise<PauseAgentChatResult> {
  return invokeCmd<PauseAgentChatResult>('pause_agent_chat', sessionId ? { sessionId } : undefined);
}

export async function resumeAgentChat(sessionId?: string): Promise<PauseAgentChatResult> {
  return invokeCmd<PauseAgentChatResult>('resume_agent_chat', sessionId ? { sessionId } : undefined);
}

export interface ModelSettingsPayload {
  connectionId?: string;
  provider: string;
  providerId?: string;
  routeId?: string;
  protocol?: string;
  apiUrl: string;
  apiKey?: string;
  clearApiKey?: boolean;
  modelName: string;
  authHeader?: string;
}

export async function checkModelSettings(payload: ModelSettingsPayload): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('check_model_settings', {
    payload: normalizeModelSettingsPayload(payload),
  });
}

export async function listModels(payload: ModelSettingsPayload): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('list_models', {
    payload: normalizeModelSettingsPayload(payload),
  });
}

export type AgentPermissionMode = 'plan' | 'default' | 'full_access';

export async function getAgentSkillMetadata(projectRoot?: string | null) {
  return invokeCmd<AgentSkillMetadata[]>(
    'get_agent_skill_metadata',
    projectRoot ? { projectRoot } : undefined,
  );
}

export async function getAgentSubagentMetadata() {
  return invokeCmd<AgentProfileMetadata[]>('get_agent_subagent_metadata');
}

export async function getAgentSkillInput(name: string) {
  return invokeCmd<AgentSkillInput>('get_agent_skill_input', { name });
}

export interface ModelUsageRecord {
  id: string;
  sessionId?: string | null;
  runId: string;
  iteration: number;
  provider: string;
  model: string;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  source: string;
  createdAt: number;
}

export interface ModelUsageSummary {
  events: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  recent: ModelUsageRecord[];
}

export interface AgentRunRecord {
  id: string;
  sessionId?: string | null;
  status: string;
  permissionMode: AgentPermissionMode | string;
  createdAt: number;
  updatedAt: number;
  finishedAt?: number | null;
  error?: string | null;
}

export interface AgentRunStepRecord {
  id: string;
  runId: string;
  stepIndex: number;
  stepType: string;
  status: string;
  summary: string;
  input: Record<string, any>;
  output: Record<string, any>;
  createdAt: number;
  finishedAt?: number | null;
}

export interface AgentToolAuditRecord {
  id: string;
  sessionId?: string | null;
  runId: string;
  iteration: number;
  toolCallId: string;
  toolName: string;
  permissionMode: string;
  policy: string;
  status: string;
  reason: string;
  createdAt: number;
}

export type PlanTaskEvidenceStatus = 'none' | 'pending' | 'verified' | 'waived' | 'failed';

export interface PlanTaskRecord {
  id: string;
  sessionId: string;
  runId?: string | null;
  parentId?: string | null;
  title: string;
  status:
    | 'pending'
    | 'doing'
    | 'running'
    | 'waiting'
    | 'verifying'
    | 'failed'
    | 'done'
    | 'blocked'
    | 'waived'
    | 'cancelled'
    | 'skipped'
    | string;
  position: number;
  source: string;
  createdAt: number;
  updatedAt: number;
  archivedAt?: number | null;
  acceptanceCriteria?: unknown;
  verify?: unknown;
  evidence?: unknown;
  evidenceStatus?: PlanTaskEvidenceStatus | string;
  active?: boolean;
  blockedReason?: string | null;
}

export interface PlanTaskPatch {
  id: string;
  title?: string;
  parentId?: string | null;
  clearParentId?: boolean;
  position?: number;
  runId?: string | null;
  clearRunId?: boolean;
  acceptanceCriteria?: unknown;
  clearAcceptanceCriteria?: boolean;
  verify?: unknown;
  clearVerify?: boolean;
}

export interface PlanChangeRecord {
  id: string;
  sessionId: string;
  runId?: string | null;
  actor: string;
  action: string;
  subjectType: string;
  subjectId: string;
  reason: string;
  before: Record<string, any>;
  after: Record<string, any>;
  createdAt: number;
}

export interface PlanTaskRunIntegrityIssue {
  taskId: string;
  sessionId: string;
  runId: string;
  issue: 'missing_run' | 'run_without_session' | 'cross_session' | string;
  runSessionId?: string | null;
}

export interface PlanTaskRunIntegrityReport {
  checkedAt: number;
  scannedTasks: number;
  issueCount: number;
  repairedCount: number;
  repairApplied: boolean;
  issues: PlanTaskRunIntegrityIssue[];
}

export interface ArtifactRecord {
  id: string;
  sessionId?: string | null;
  runId?: string | null;
  kind: string;
  title: string;
  path?: string | null;
  operation: string;
  status: string;
  summary: string;
  metadata: Record<string, any>;
  createdAt: number;
  updatedAt: number;
}

export async function getModelUsageSummary(sessionId?: string) {
  return invokeCmd<ModelUsageSummary>('get_model_usage_summary', sessionId ? { sessionId } : undefined);
}

export async function getAgentRuns(sessionId?: string, limit = 20) {
  return invokeCmd<AgentRunRecord[]>('get_agent_runs', { sessionId, limit });
}

export async function getAgentRunSteps(runId: string) {
  return invokeCmd<AgentRunStepRecord[]>('get_agent_run_steps', { runId });
}

/** P1-3/OS-2: one entry in a run's unified replay timeline. `detail` is the full real
 * source record (step/browser/tool/usage/verify/permission) — real data, not faked.
 * For `permission`, `status` is the decision (allowed/needs_confirm/denied). */
export interface RunTimelineEntry {
  kind: 'step' | 'browser' | 'tool' | 'usage' | 'verify' | 'permission' | string;
  id: string;
  at: number;
  finishedAt?: number | null;
  seq: number;
  label: string;
  status?: string | null;
  detail: Record<string, any>;
}

/** P1-3: a paginated slice of a run's replay timeline. `total` is the run's full
 * event count, so the caller can page through the entire run (full replay). */
export interface RunTimeline {
  runId: string;
  run?: AgentRunRecord | null;
  total: number;
  offset: number;
  limit: number;
  entries: RunTimelineEntry[];
}

/** P1-3/OS-2: replay a run's full event timeline (step + browser + tool + usage + verify) in
 * chronological order. Page through the whole run via `limit`/`offset` + `total`. */
export async function getAgentRunTimeline(runId: string, limit = 200, offset = 0) {
  return invokeCmd<RunTimeline>('get_agent_run_timeline', { runId, limit, offset });
}

export interface BrowserDomSummary {
  totalElements: number;
  interactiveElements: number;
  links: number;
  iframes: number;
  shadowRoots: number;
  images: number;
  textChars: number;
  emptyPage: boolean;
  skeletonLike: boolean;
  truncatedElements: number;
}

export interface BrowserJudgeResult {
  status: string;
  reason: string;
  blocksCompletion: boolean;
  evidence: string[];
}

export interface BrowserAgentStepRecord {
  id: string;
  sessionId?: string | null;
  runId?: string | null;
  stepIndex: number;
  action: string;
  target?: string | null;
  status: string;
  title?: string | null;
  url?: string | null;
  screenshotPath?: string | null;
  domSummary: Record<string, any>;
  actionJson: Record<string, any>;
  resultJson: Record<string, any>;
  fingerprint: string;
  judge: Record<string, any>;
  loopDetected: boolean;
  createdAt: number;
}

export async function getBrowserAgentSteps(args: {
  runId?: string | null;
  sessionId?: string | null;
  limit?: number;
}) {
  return invokeCmd<BrowserAgentStepRecord[]>('get_browser_agent_steps', args);
}

export interface AgentGraphNodeSpec {
  nodeKey: string;
  kind: string;
  title: string;
  maxAttempts?: number | null;
  input?: unknown;
}

export interface AgentGraphEdgeSpec {
  fromNodeKey: string;
  toNodeKey: string;
  condition?: string | null;
}

export interface CreateAgentGraphSpec {
  id?: string | null;
  sessionId?: string | null;
  sourceRunId?: string | null;
  goal: string;
  nodes: AgentGraphNodeSpec[];
  edges?: AgentGraphEdgeSpec[];
}

export interface AgentGraphRunRecord {
  id: string;
  sessionId?: string | null;
  sourceRunId?: string | null;
  goal: string;
  status: string;
  createdAt: number;
  updatedAt: number;
  finishedAt?: number | null;
  error?: string | null;
}

export interface AgentGraphNodeRecord {
  id: string;
  graphRunId: string;
  nodeKey: string;
  kind: string;
  title: string;
  status: string;
  attempt: number;
  maxAttempts: number;
  input: Record<string, any>;
  output: Record<string, any>;
  error?: string | null;
  createdAt: number;
  updatedAt: number;
  startedAt?: number | null;
  finishedAt?: number | null;
}

export interface AgentGraphEdgeRecord {
  id: string;
  graphRunId: string;
  fromNodeId: string;
  toNodeId: string;
  condition?: string | null;
  createdAt: number;
}

export interface AgentGraphCheckpointRecord {
  id: string;
  graphRunId: string;
  nodeId?: string | null;
  state: Record<string, any>;
  createdAt: number;
}

export interface AgentGraphSnapshot {
  run: AgentGraphRunRecord;
  nodes: AgentGraphNodeRecord[];
  edges: AgentGraphEdgeRecord[];
  checkpoints: AgentGraphCheckpointRecord[];
}

export async function createAgentGraphRun(spec: CreateAgentGraphSpec) {
  return invokeCmd<AgentGraphSnapshot>('create_agent_graph_run', { spec });
}

export async function getAgentGraphSnapshot(graphRunId: string) {
  return invokeCmd<AgentGraphSnapshot>('get_agent_graph_snapshot', { graphRunId });
}

export async function pauseAgentGraphRun(graphRunId: string, reason: string) {
  return invokeCmd<AgentGraphSnapshot>('pause_agent_graph_run', { graphRunId, reason });
}

export async function resumeAgentGraphRun(graphRunId: string, reason: string) {
  return invokeCmd<AgentGraphSnapshot>('resume_agent_graph_run', { graphRunId, reason });
}

export interface TeamParticipantSpec {
  name: string;
  role: string;
  model?: string | null;
  toolScope?: Record<string, any>;
}

export interface CreateTeamRunSpec {
  id?: string | null;
  sessionId?: string | null;
  sourceRunId?: string | null;
  goal: string;
  maxRounds?: number | null;
  participants: TeamParticipantSpec[];
}

export interface TeamRunRecord {
  id: string;
  sessionId?: string | null;
  sourceRunId?: string | null;
  goal: string;
  status: string;
  maxRounds: number;
  terminationReason?: string | null;
  createdAt: number;
  updatedAt: number;
  finishedAt?: number | null;
}

export interface TeamParticipantRecord {
  id: string;
  teamRunId: string;
  name: string;
  role: string;
  model?: string | null;
  toolScope: Record<string, any>;
  status: string;
  createdAt: number;
  updatedAt: number;
}

export interface TeamMessageRecord {
  id: string;
  teamRunId: string;
  participantId?: string | null;
  role: string;
  messageType: string;
  content: string;
  metadata: Record<string, any>;
  createdAt: number;
}

export interface HandoffContract {
  task: string;
  expectedOutput?: string;
  allowedTools?: string[];
  canMarkComplete?: boolean;
}

export interface HandoffRequestRecord {
  id: string;
  teamRunId: string;
  fromParticipantId?: string | null;
  toParticipantId: string;
  status: string;
  reason: string;
  contract: Record<string, any>;
  result: Record<string, any>;
  createdAt: number;
  resolvedAt?: number | null;
}

export interface TeamRunSnapshot {
  run: TeamRunRecord;
  participants: TeamParticipantRecord[];
  messages: TeamMessageRecord[];
  handoffs: HandoffRequestRecord[];
}

export interface TeamTerminationVerdict {
  status: string;
  reason: string;
  shouldStop: boolean;
}

export async function createTeamRun(spec: CreateTeamRunSpec) {
  return invokeCmd<TeamRunSnapshot>('create_team_run', { spec });
}

export async function getTeamRunSnapshot(teamRunId: string) {
  return invokeCmd<TeamRunSnapshot>('get_team_run_snapshot', { teamRunId });
}

export async function appendTeamMessage(args: {
  teamRunId: string;
  participantId?: string | null;
  messageType: string;
  content: string;
  metadata?: Record<string, any>;
}) {
  return invokeCmd<TeamMessageRecord>('append_team_message', args);
}

export async function createHandoffRequest(args: {
  teamRunId: string;
  fromParticipantId?: string | null;
  toParticipantId: string;
  reason: string;
  contract: HandoffContract;
}) {
  return invokeCmd<HandoffRequestRecord>('create_handoff_request', args);
}

export async function resolveHandoffRequest(args: {
  handoffId: string;
  status: string;
  result: Record<string, any>;
}) {
  return invokeCmd<HandoffRequestRecord>('resolve_handoff_request', args);
}

export async function applyTeamTermination(teamRunId: string) {
  return invokeCmd<TeamTerminationVerdict>('apply_team_termination', { teamRunId });
}

export interface TeamExecutionOptions {
  maxSteps?: number | null;
  defaultTokenBudget?: number | null;
  roleTokenBudgets?: Record<string, number>;
  requireMainReview?: boolean;
}

export interface TeamExecutionStep {
  order: number;
  participantId: string;
  name: string;
  role: string;
  model?: string | null;
  action: string;
  tokenBudget: number;
  toolScope: unknown;
  status: string;
  reason: string;
}

export interface TeamExecutionPlan {
  teamRunId: string;
  runStatus: string;
  paused: boolean;
  requiresMainReview: boolean;
  stepCount: number;
  steps: TeamExecutionStep[];
  reason: string;
  builtAt: number;
}

export async function scheduleTeamExecutionPlan(
  teamRunId: string,
  options: TeamExecutionOptions = {},
) {
  return invokeCmd<TeamExecutionPlan>('schedule_team_execution_plan', { teamRunId, options });
}

export async function pauseTeamExecution(teamRunId: string, reason: string) {
  return invokeCmd<TeamExecutionPlan>('pause_team_execution', { teamRunId, reason });
}

export async function resumeTeamExecution(teamRunId: string, reason: string) {
  return invokeCmd<TeamExecutionPlan>('resume_team_execution', { teamRunId, reason });
}

export interface AddKnowledgeItemPayload {
  id?: string | null;
  scope: string;
  source: string;
  trust: string;
  title: string;
  text: string;
  confidence?: number | null;
  expiresAt?: number | null;
  embeddingRef?: string | null;
}

export interface KnowledgeItemRecord extends AddKnowledgeItemPayload {
  id: string;
  enabled: boolean;
  confidence: number;
  embeddingRef?: string | null;
  createdAt: number;
  updatedAt: number;
  deletedAt?: number | null;
}

export interface RetrievalHitRecord {
  itemId: string;
  scope: string;
  source: string;
  trust: string;
  title: string;
  snippet: string;
  score: number;
  confidence: number;
  reason: string;
  embeddingRef?: string | null;
  createdAt: number;
}

export interface KnowledgeRecallRequest {
  query: string;
  scope?: string | null;
  limit?: number | null;
}

export interface RetrievalContext {
  hits: RetrievalHitRecord[];
  systemNote?: string | null;
}

export async function addKnowledgeItem(payload: AddKnowledgeItemPayload) {
  return invokeCmd<KnowledgeItemRecord>('add_knowledge_item', { payload });
}

export async function searchKnowledge(request: KnowledgeRecallRequest) {
  return invokeCmd<RetrievalContext>('search_knowledge', { request });
}

export async function deleteKnowledgeItem(id: string) {
  return invokeCmd<KnowledgeItemRecord>('delete_knowledge_item', { id });
}

export interface KnowledgeConnectorItem {
  scope: string;
  source: string;
  trust: string;
  title: string;
  text: string;
  confidence?: number | null;
  expiresAt?: number | null;
  embeddingRef?: string | null;
}

export interface KnowledgeConnectorIngestRequest {
  connectorId: string;
  items?: KnowledgeConnectorItem[];
}

export interface KnowledgeConnectorIngestReport {
  connectorId: string;
  inserted: number;
  skipped: number;
  itemIds: string[];
  warnings: string[];
}

export interface MemoryWriteEvent {
  scope: string;
  eventType: string;
  title: string;
  text: string;
  runId?: string | null;
  toolName?: string | null;
  success?: boolean | null;
}

export async function ingestConnectorKnowledgeItems(request: KnowledgeConnectorIngestRequest) {
  return invokeCmd<KnowledgeConnectorIngestReport>('ingest_connector_knowledge_items', { request });
}

export async function writeAgentMemoryEvent(event: MemoryWriteEvent) {
  return invokeCmd<KnowledgeItemRecord>('write_agent_memory_event', { event });
}

export interface WorkspaceLifecycleSpec {
  id?: string | null;
  sessionId?: string | null;
  runId?: string | null;
  rootPath: string;
  sandboxBackend?: string | null;
  setupScript?: string | null;
}

export interface WorkspaceLifecycleRecord {
  id: string;
  sessionId?: string | null;
  runId?: string | null;
  rootPath: string;
  status: string;
  setupStatus: string;
  sandboxBackend: string;
  sandboxStatus: string;
  fallbackReason?: string | null;
  setupScript?: string | null;
  audit: Record<string, any>;
  createdAt: number;
  updatedAt: number;
  archivedAt?: number | null;
}

export interface WorkspaceSetupEventRecord {
  id: string;
  workspaceId: string;
  stage: string;
  status: string;
  command?: string | null;
  exitCode?: number | null;
  outputTail: string;
  reason: string;
  createdAt: number;
}

export interface WorkspaceLifecycleSnapshot {
  workspace: WorkspaceLifecycleRecord;
  events: WorkspaceSetupEventRecord[];
}

export interface WorkspaceCwdVerdict {
  allowed: boolean;
  workspaceRoot: string;
  cwd: string;
  reason: string;
}

export interface WorkspaceSetupRunOptions {
  timeoutMs?: number | null;
}

export interface WorkspaceGitHookSpec {
  hookName: string;
  command: string;
  overwrite?: boolean;
}

export interface WorkspaceGitHookInstallReport {
  workspaceId: string;
  hookName: string;
  hookPath: string;
  installed: boolean;
  reason: string;
}

export async function createWorkspaceLifecycle(spec: WorkspaceLifecycleSpec) {
  return invokeCmd<WorkspaceLifecycleRecord>('create_workspace_lifecycle', { spec });
}

export async function getWorkspaceLifecycleSnapshot(workspaceId: string) {
  return invokeCmd<WorkspaceLifecycleSnapshot>('get_workspace_lifecycle_snapshot', { workspaceId });
}

export async function validateWorkspaceCwd(workspaceId: string, cwd: string) {
  return invokeCmd<WorkspaceCwdVerdict>('validate_workspace_cwd', { workspaceId, cwd });
}

export async function runWorkspaceSetupScript(
  workspaceId: string,
  options: WorkspaceSetupRunOptions = {},
) {
  return invokeCmd<WorkspaceLifecycleSnapshot>('run_workspace_setup_script', {
    workspaceId,
    options,
  });
}

export async function validateWorkspaceCommandBinding(workspaceId: string, cwd: string) {
  return invokeCmd<WorkspaceCwdVerdict>('validate_workspace_command_binding', { workspaceId, cwd });
}

export async function installWorkspaceGitHook(workspaceId: string, spec: WorkspaceGitHookSpec) {
  return invokeCmd<WorkspaceGitHookInstallReport>('install_workspace_git_hook', {
    workspaceId,
    spec,
  });
}

export interface TrajectoryExportOptions {
  runId: string;
  includePayloads?: boolean;
  redactSecrets?: boolean;
  limit?: number | null;
}

export interface TrajectoryEvent {
  eventType: string;
  runId: string;
  sourceId: string;
  timestamp: number;
  sequence: number;
  label: string;
  status?: string | null;
  redactionState: string;
  payload: unknown;
}

export interface TrajectoryExport {
  runId: string;
  format: string;
  eventCount: number;
  totalAvailable: number;
  truncated: boolean;
  redactionState: string;
  findingsRedacted: number;
  replaySafe: boolean;
  exportedAt: number;
  events: TrajectoryEvent[];
}

export interface ReplayFrame {
  index: number;
  eventType: string;
  sourceId: string;
  timestamp: number;
  summary: string;
  wouldMutate: boolean;
  payload: unknown;
}

export interface ReplayReport {
  runId: string;
  frameCount: number;
  externalCallsBlocked: boolean;
  workspaceMutationsBlocked: boolean;
  findingsRedacted: number;
  frames: ReplayFrame[];
}

export interface TrajectoryEvalReport {
  runId: string;
  frameCount: number;
  mutatingFrameCount: number;
  verificationFrameCount: number;
  completionClaimCount: number;
  falseCompletionRisk: string;
  reasons: string[];
}

export async function exportAgentRunTrajectory(options: TrajectoryExportOptions) {
  return invokeCmd<TrajectoryExport>('export_agent_run_trajectory', { options });
}

export async function replayAgentRunTrajectory(runId: string) {
  return invokeCmd<ReplayReport>('replay_agent_run_trajectory', { runId });
}

export async function evaluateAgentRunTrajectory(runId: string) {
  return invokeCmd<TrajectoryEvalReport>('evaluate_agent_run_trajectory', { runId });
}

export interface ExternalArtifactInput {
  artifactType: string;
  title: string;
  uri?: string | null;
  metadata?: unknown;
}

export interface ExternalTask {
  protocol: string;
  version?: string;
  externalTaskId: string;
  sessionId?: string | null;
  input: unknown;
  artifacts?: ExternalArtifactInput[];
  permissionMode?: string;
}

export interface ArtifactRef {
  artifactId: string;
  artifactType: string;
  title: string;
  uri?: string | null;
  metadata: unknown;
}

export interface ProtocolRunMapping {
  protocol: string;
  version: string;
  externalTaskId: string;
  runId: string;
  status: string;
  artifactRefs: ArtifactRef[];
  audit: unknown;
}

export interface ProtocolCompatibilityEntry {
  protocol: string;
  supportedVersions: string[];
  taskMapping: boolean;
  lifecycle: boolean;
  streaming: boolean;
  cancellation: boolean;
  notes: string[];
}

export interface ProtocolLifecycleUpdate {
  protocol: string;
  externalTaskId: string;
  status: string;
  message?: string | null;
  output?: unknown;
}

export interface ProtocolStreamEvent {
  sequence: number;
  eventType: string;
  status: string;
  payload: unknown;
  at: number;
}

export async function createExternalAgentTask(task: ExternalTask) {
  return invokeCmd<ProtocolRunMapping>('create_external_agent_task', { task });
}

export async function getExternalAgentTaskMapping(protocol: string, externalTaskId: string) {
  return invokeCmd<ProtocolRunMapping>('get_external_agent_task_mapping', {
    protocol,
    externalTaskId,
  });
}

export async function getProtocolCompatibilityMatrix() {
  return invokeCmd<ProtocolCompatibilityEntry[]>('get_protocol_compatibility_matrix');
}

export async function updateExternalAgentTaskLifecycle(update: ProtocolLifecycleUpdate) {
  return invokeCmd<ProtocolRunMapping>('update_external_agent_task_lifecycle', { update });
}

export async function cancelExternalAgentTask(
  protocol: string,
  externalTaskId: string,
  reason: string,
) {
  return invokeCmd<ProtocolRunMapping>('cancel_external_agent_task', {
    protocol,
    externalTaskId,
    reason,
  });
}

export async function appendExternalAgentTaskStreamEvent(args: {
  protocol: string;
  externalTaskId: string;
  eventType: string;
  payload: unknown;
}) {
  return invokeCmd<ProtocolStreamEvent[]>('append_external_agent_task_stream_event', args);
}

export async function listExternalAgentTaskStreamEvents(protocol: string, externalTaskId: string) {
  return invokeCmd<ProtocolStreamEvent[]>('list_external_agent_task_stream_events', {
    protocol,
    externalTaskId,
  });
}

export interface PluginQualityGateRequest {
  pluginId: string;
  devMode?: boolean;
}

export interface PluginQualityGate {
  pluginId: string;
  status: string;
  canEnable: boolean;
  risk: string;
  modelTierHint: string;
  requiredEval: boolean;
  reasons: string[];
  checkedAt: number;
}

export interface PluginEvalRegistryEntry {
  pluginId: string;
  version: string;
  required: boolean;
  suiteId?: string | null;
  commands: string[];
  status: string;
  reasons: string[];
}

export interface SkillVersionRecord {
  pluginId: string;
  skillId: string;
  version: string;
  source: string;
  risk: string;
}

export interface TeamPresetRolePermission {
  role: string;
  allowedCapabilities: string[];
  deniedPermissions: string[];
  risk: string;
}

export interface TeamPresetPermissionReport {
  pluginId: string;
  canBind: boolean;
  roles: TeamPresetRolePermission[];
  reasons: string[];
}

export interface PluginPackageRecord {
  id: string;
  name: string;
  version: string;
  source: string;
  description: string;
  trusted: boolean;
  enabled: boolean;
  risk: string;
  permissions: unknown;
  capabilities: unknown;
  manifest: unknown;
  installed_at?: number | null;
  updated_at?: number | null;
  installedAt?: number | null;
  updatedAt?: number | null;
}

export interface PluginCapabilityEventRecord {
  id: string;
  pluginId?: string;
  plugin_id?: string;
  capabilityId?: string;
  capability_id?: string;
  action: string;
  status: string;
  risk: string;
  reason: string;
  input: unknown;
  output: unknown;
  createdAt?: number;
  created_at?: number;
}

export interface ToolResult {
  status: 'success' | 'warning' | 'error' | string;
  summary: string;
  data: unknown;
  next_actions?: string[];
  nextActions?: string[];
  recoverable?: boolean;
}

export interface GitCommandArgs {
  cwd?: string;
}

export interface GitDiffArgs extends GitCommandArgs {
  staged?: boolean;
  stat?: boolean;
  refs?: string[];
  paths?: string[];
}

export interface GitLogArgs extends GitCommandArgs {
  limit?: number;
  reference?: string;
  paths?: string[];
}

export interface GitShowArgs extends GitCommandArgs {
  reference: string;
  stat?: boolean;
}

export interface GitStageArgs extends GitCommandArgs {
  paths?: string[];
  all?: boolean;
  confirmed?: boolean;
}

export interface GitCommitArgs extends GitCommandArgs {
  message: string;
  confirmed?: boolean;
}

export interface GitCreateBranchArgs extends GitCommandArgs {
  branch: string;
  startPoint?: string;
  confirmed?: boolean;
}

export interface GitPushArgs extends GitCommandArgs {
  remote: string;
  branch: string;
  setUpstream?: boolean;
  confirmed?: boolean;
}

export async function gitStatus(cwd?: string) {
  return invokeCmd<ToolResult>('git_status', { cwd });
}

export async function gitDiff(args: GitDiffArgs = {}) {
  return invokeCmd<ToolResult>('git_diff', { ...args });
}

export async function gitLog(args: GitLogArgs = {}) {
  return invokeCmd<ToolResult>('git_log', { ...args });
}

export async function gitShow(args: GitShowArgs) {
  return invokeCmd<ToolResult>('git_show', { ...args });
}

export async function gitStage(args: GitStageArgs) {
  return invokeCmd<ToolResult>('git_stage', { ...args });
}

export async function gitCommit(args: GitCommitArgs) {
  return invokeCmd<ToolResult>('git_commit', { ...args });
}

export async function gitCreateBranch(args: GitCreateBranchArgs) {
  return invokeCmd<ToolResult>('git_create_branch', { ...args });
}

export async function gitPush(args: GitPushArgs) {
  return invokeCmd<ToolResult>('git_push', { ...args });
}

export async function evaluatePluginQualityGate(request: PluginQualityGateRequest) {
  return invokeCmd<PluginQualityGate>('evaluate_plugin_quality_gate', { request });
}

export async function getPluginEvalRegistryEntry(pluginId: string) {
  return invokeCmd<PluginEvalRegistryEntry>('get_plugin_eval_registry_entry', { pluginId });
}

export async function getSkillVersionRegistry(pluginId: string) {
  return invokeCmd<SkillVersionRecord[]>('get_skill_version_registry', { pluginId });
}

export async function getTeamPresetPermissionReport(pluginId: string) {
  return invokeCmd<TeamPresetPermissionReport>('get_team_preset_permission_report', { pluginId });
}

export async function listPluginPackages() {
  return invokeCmd<PluginPackageRecord[]>('list_plugin_packages');
}

export async function installPluginPackage(args: {
  manifest: Record<string, unknown>;
  enabled?: boolean;
  trusted?: boolean;
  confirmed?: boolean;
}) {
  return invokeCmd<ToolResult>('install_plugin_package', args);
}

export async function setPluginPackageEnabled(id: string, enabled: boolean) {
  return invokeCmd<PluginPackageRecord>('set_plugin_package_enabled', { id, enabled });
}

export async function invokePluginCapability(args: {
  pluginId: string;
  capabilityId: string;
  input?: Record<string, unknown>;
  confirmed?: boolean;
}) {
  return invokeCmd<ToolResult>('invoke_plugin_capability', args);
}

export async function getPluginCapabilityEvents(pluginId?: string, limit = 80) {
  return invokeCmd<PluginCapabilityEventRecord[]>('get_plugin_capability_events', {
    pluginId,
    limit,
  });
}

export interface CodeIntelligenceRequest {
  workspaceRoot: string;
  documentPath?: string | null;
  lspDiagnostics?: unknown;
}

export interface LspBackendStatus {
  available: boolean;
  backend: string;
  reason: string;
}

export interface DiagnosticSummary {
  uri: string;
  severity: string;
  message: string;
  line: number;
  character: number;
  source?: string | null;
}

export interface CodeIntelligenceReport {
  workspaceRoot: string;
  documentPath?: string | null;
  backend: LspBackendStatus;
  diagnostics: DiagnosticSummary[];
  bounded: boolean;
}

export interface LspSessionSpec {
  workspaceRoot: string;
  language: string;
  serverCommand?: string | null;
}

export interface LspSessionPlan {
  workspaceRoot: string;
  language: string;
  command: string;
  backend: LspBackendStatus;
  bounded: boolean;
}

export async function inspectCodeIntelligenceReport(request: CodeIntelligenceRequest) {
  return invokeCmd<CodeIntelligenceReport>('inspect_code_intelligence_report', { request });
}

export async function prepareLspSessionPlan(request: LspSessionSpec) {
  return invokeCmd<LspSessionPlan>('prepare_lsp_session_plan', { request });
}

export interface ModelCostEstimate {
  provider: string;
  model: string;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  priceKnown: boolean;
  estimatedCostUsd?: number | null;
  rule?: unknown;
}

export interface ProviderTokenEstimate {
  provider: string;
  model: string;
  tokenizer: string;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  warnings: string[];
}

export interface ModelTextCostEstimate {
  tokens: ProviderTokenEstimate;
  cost: ModelCostEstimate;
}

export interface ModelQualityEvent {
  id: string;
  provider: string;
  model: string;
  runId?: string | null;
  eventType: string;
  severity: string;
  weight: number;
  reason: string;
  createdAt: number;
}

export interface RecordModelQualityEventRequest {
  provider: string;
  model: string;
  runId?: string | null;
  eventType: string;
  severity?: string | null;
  weight?: number | null;
  reason: string;
}

export interface RouteEconomicsDecision {
  provider: string;
  model: string;
  estimatedCostUsd?: number | null;
  qualityPenalty: number;
  recommendation: string;
  reasons: string[];
}

export interface ModelRouteDecisionAudit {
  id: string;
  tier: string;
  selectedConnectionId?: string | null;
  selectedProviderId?: string | null;
  selectedModel?: string | null;
  reasonCodes: string[];
  candidateCount: number;
  createdAt: number;
}

export async function estimateModelCost(args: {
  provider: string;
  model: string;
  inputTokens: number;
  outputTokens: number;
}) {
  return invokeCmd<ModelCostEstimate>('estimate_model_cost', args);
}

export async function estimateModelTextCost(args: {
  provider: string;
  model: string;
  inputText: string;
  outputText?: string;
}) {
  return invokeCmd<ModelTextCostEstimate>('estimate_model_text_cost', args);
}

export async function recordModelQualityEvent(request: RecordModelQualityEventRequest) {
  return invokeCmd<ModelQualityEvent>('record_model_quality_event', { request });
}

export async function getModelQualityEvents() {
  return invokeCmd<ModelQualityEvent[]>('get_model_quality_events');
}

export async function getModelRouteDecisions() {
  return invokeCmd<ModelRouteDecisionAudit[]>('get_model_route_decisions');
}

export async function explainRouteEconomics(args: {
  provider: string;
  model: string;
  inputTokens: number;
  outputTokens: number;
}) {
  return invokeCmd<RouteEconomicsDecision>('explain_route_economics', args);
}

export interface NodeExecutionTrace {
  nodeId: string;
  nodeKey: string;
  kind: string;
  status: string;
  attempt: number;
  startedAt?: number | null;
  finishedAt?: number | null;
  input: unknown;
  output: unknown;
  error?: string | null;
  redactionState: string;
  findingsRedacted: number;
}

export interface WorkflowTraceReport {
  graphRunId: string;
  nodeCount: number;
  redactionState: string;
  findingsRedacted: number;
  traces: NodeExecutionTrace[];
}

export interface QueuedGraphRun {
  id: string;
  graphRunId: string;
  status: string;
  priority: number;
  reason: string;
  createdAt: number;
  updatedAt: number;
}

export interface WorkflowQueueControl {
  paused: boolean;
  reason: string;
  updatedAt: number;
}

export async function getAgentGraphNodeTraces(graphRunId: string) {
  return invokeCmd<WorkflowTraceReport>('get_agent_graph_node_traces', { graphRunId });
}

export async function enqueueAgentGraphRun(graphRunId: string, priority?: number | null) {
  return invokeCmd<QueuedGraphRun>('enqueue_agent_graph_run', { graphRunId, priority });
}

export async function abortQueuedAgentGraphRun(queueId: string, reason: string) {
  return invokeCmd<QueuedGraphRun>('abort_queued_agent_graph_run', { queueId, reason });
}

export async function listAgentGraphQueue() {
  return invokeCmd<QueuedGraphRun[]>('list_agent_graph_queue');
}

export async function setAgentGraphQueuePaused(paused: boolean, reason: string) {
  return invokeCmd<WorkflowQueueControl>('set_agent_graph_queue_paused', { paused, reason });
}

export async function getAgentGraphQueueControl() {
  return invokeCmd<WorkflowQueueControl>('get_agent_graph_queue_control');
}

export interface RunDiffFile {
  path: string;
  status: 'created' | 'modified' | 'deleted' | 'unchanged' | string;
  additions: number;
  deletions: number;
  statsAccurate: boolean;
  diffText?: string | null;
  diffTruncated: boolean;
  unavailableReason?: string | null;
  beforeHash?: string | null;
  afterHash?: string | null;
  firstCheckpointId: string;
  lastCheckpointId: string;
  checkpointCount: number;
  createdAt: number;
  updatedAt: number;
}

export interface RunDiff {
  runId: string;
  totalFiles: number;
  returnedFiles: number;
  truncated: boolean;
  files: RunDiffFile[];
}

/** P4-1 backend diff feed. Uses checkpoint snapshots, not current file reads. */
export async function getAgentRunDiff(runId: string, limit = 200) {
  return invokeCmd<RunDiff>('get_agent_run_diff', { runId, limit });
}

export interface StatusSemantic {
  domain: string;
  status: string;
  tone: 'success' | 'warning' | 'danger' | 'info' | 'neutral' | 'muted' | string;
  label: string;
  isTerminal: boolean;
  blocksCompletion: boolean;
}

export interface RunTerminalEntry {
  id: string;
  sourceId: string;
  sourceKind: string;
  at: number;
  finishedAt?: number | null;
  seq: number;
  command: string;
  cwd?: string | null;
  shell?: string | null;
  status: string;
  exitCode?: number | null;
  stdoutTail?: string | null;
  stderrTail?: string | null;
  summary: string;
  toolCallId?: string | null;
  truncated: boolean;
}

export interface RunTerminalFeed {
  runId: string;
  total: number;
  offset: number;
  limit: number;
  entries: RunTerminalEntry[];
}

/** P4-2 architecture feed. Rows are derived only from real command payloads. */
export async function getAgentRunTerminal(runId: string, limit = 200, offset = 0) {
  return invokeCmd<RunTerminalFeed>('get_agent_run_terminal', { runId, limit, offset });
}

export interface RunAuditEntry {
  id: string;
  sourceId: string;
  sourceKind: string;
  at: number;
  finishedAt?: number | null;
  seq: number;
  category: string;
  label: string;
  status?: string | null;
  semantic: StatusSemantic;
  risk?: string | null;
  actor?: string | null;
  reason?: string | null;
  detail: Record<string, any>;
}

export interface RunAuditFeed {
  runId: string;
  total: number;
  offset: number;
  limit: number;
  entries: RunAuditEntry[];
}

/** P4-3 architecture feed. Keeps original audit source records in `detail`. */
export async function getAgentRunAudit(runId: string, limit = 200, offset = 0) {
  return invokeCmd<RunAuditFeed>('get_agent_run_audit', { runId, limit, offset });
}

export interface RunProgressSummary {
  runId: string;
  runStatus?: string | null;
  stage: string;
  semantic: StatusSemantic;
  latestMessage?: string | null;
  eventCounts: Record<string, number>;
  startedAt?: number | null;
  updatedAt?: number | null;
  finishedAt?: number | null;
  lastEventAt?: number | null;
  terminal: boolean;
  hasVerification: boolean;
  failedVerificationCount: number;
  pendingPermissionCount: number;
}

export async function getAgentRunProgress(runId: string) {
  return invokeCmd<RunProgressSummary>('get_agent_run_progress', { runId });
}

export async function getAgentStatusSemantic(domain: string, status: string) {
  return invokeCmd<StatusSemantic>('get_agent_status_semantic', { domain, status });
}

export interface EvalCommand {
  command: string;
  cwd?: string | null;
  timeoutMs?: number | null;
  required: boolean;
}

export interface EvalVerifier {
  commands: EvalCommand[];
  evidence: string[];
  successCriteria: string[];
}

export interface EvalCase {
  id: string;
  title: string;
  category: string;
  tags: string[];
  prompt: string;
  setup: string[];
  allowedProviders: string[];
  expected: string[];
  forbidden: string[];
  verifier: EvalVerifier;
}

export interface EvalExitGate {
  minCases: number;
  minPassRate: number;
  maxFalseCompletionRate: number;
  requireAllCritical: boolean;
  requiredTags: string[];
}

export interface EvalSuite {
  id: string;
  name: string;
  kind: string;
  description: string;
  exitGate: EvalExitGate;
  cases: EvalCase[];
}

export interface EvalCaseOutcome {
  caseId: string;
  passed: boolean;
  verified: boolean;
  falseCompletion?: boolean;
  blocked?: boolean;
  provider?: string | null;
  notes?: string | null;
}

export interface EvalSuiteReport {
  suiteId: string;
  totalCases: number;
  evaluatedCases: number;
  passedCases: number;
  verifiedCases: number;
  falseCompletionCases: number;
  blockedCases: number;
  passRate: number;
  falseCompletionRate: number;
  missingOutcomes: string[];
  unknownOutcomes: string[];
  criticalFailures: string[];
  gateFailures: string[];
  passed: boolean;
}

export interface EvalCommandResult {
  command: string;
  cwd: string;
  required: boolean;
  status: string;
  exitCode?: number | null;
  stdoutTail: string;
  stderrTail: string;
  startedAt: number;
  finishedAt: number;
  durationMs: number;
  timedOut: boolean;
}

export interface EvalCaseRunResult {
  caseId: string;
  title: string;
  category: string;
  tags: string[];
  status: string;
  outcome: EvalCaseOutcome;
  commands: EvalCommandResult[];
}

export interface EvalRunReport {
  id: string;
  suiteId: string;
  startedAt: number;
  finishedAt: number;
  durationMs: number;
  cwd: string;
  caseResults: EvalCaseRunResult[];
  score: EvalSuiteReport;
}

export async function getAgentEvalSuites() {
  return invokeCmd<EvalSuite[]>('get_agent_eval_suites');
}

export async function scoreAgentEvalSuite(suiteId: string, outcomes: EvalCaseOutcome[]) {
  return invokeCmd<EvalSuiteReport>('score_agent_eval_suite', { suiteId, outcomes });
}

export async function runAgentEvalSuiteVerifiers(
  suiteId: string,
  options?: {
    caseIds?: string[];
    cwd?: string | null;
    claimedComplete?: boolean;
  },
) {
  return invokeCmd<EvalRunReport>('run_agent_eval_suite_verifiers', {
    suiteId,
    caseIds: options?.caseIds,
    cwd: options?.cwd,
    claimedComplete: options?.claimedComplete,
  });
}

/** P0-4 / P1-7: one entry in the permission-decision ledger (谁批/为什么/何时).
 * `decision` is allowed | needs_confirm | denied; `decidedBy` is
 * gate | policy | skill | hard_rule | user (user = an explicit confirmation). */
export interface PermissionDecisionRecord {
  id: string;
  sessionId?: string | null;
  runId: string;
  iteration: number;
  toolCallId: string;
  subject: string;
  action: string;
  risk: string;
  mode: string;
  decision: 'allowed' | 'needs_confirm' | 'denied' | string;
  reason: string;
  decidedBy: 'gate' | 'policy' | 'skill' | 'hard_rule' | 'user' | string;
  createdAt: number;
}

export async function getAgentPermissionDecisions(runId: string, limit = 200) {
  return invokeCmd<PermissionDecisionRecord[]>('get_agent_permission_decisions', { runId, limit });
}

/** P1-7: resolve a pending needs_confirm action — write the user's approve/deny
 * back into the permission-decision ledger as a `user` decision. The confirmation
 * card UI that collects this is deferred; this is its backend write-back. */
export interface ResolvePermissionConfirmationInput {
  runId: string;
  toolCallId: string;
  subject: string;
  action: string;
  risk: string;
  mode: string;
  approved: boolean;
  iteration?: number;
  impact?: string;
  sessionId?: string;
}

export async function resolvePermissionConfirmation(input: ResolvePermissionConfirmationInput) {
  return invokeCmd<PermissionDecisionRecord>('resolve_permission_confirmation', { ...input });
}

/** P1-6: structured classification of a user message. Rule-based, deterministic. */
export interface UserIntent {
  intentType: 'chat' | 'question' | 'task' | 'edit' | 'debug' | 'review' | string;
  needsAction: boolean;
  needsClarification: boolean;
  urgency: 'low' | 'normal' | 'high' | string;
  signals: string[];
}

/** P1-6: structured project perception — stack and how to build/test, with
 * dependency/build dirs excluded (never dragged into context). */
export interface ProjectSnapshot {
  root: string;
  languages: string[];
  packageManagers: string[];
  importantFiles: string[];
  entryPoints: string[];
  testCommands: string[];
  ignoredPatterns: string[];
}

/** P1-6: classify a user message into a structured intent. */
export async function classifyUserIntent(message: string) {
  return invokeCmd<UserIntent>('classify_user_intent', { message });
}

/** P1-6: scan a project root into a structured snapshot (defaults to process cwd). */
export async function getProjectSnapshot(path?: string) {
  return invokeCmd<ProjectSnapshot>('get_project_snapshot', path ? { path } : undefined);
}

export async function getAgentToolAuditEvents(sessionId?: string, limit = 80) {
  return invokeCmd<AgentToolAuditRecord[]>('get_agent_tool_audit_events', { sessionId, limit });
}

export async function retryAgentRun(runId: string, instruction?: string) {
  return invokeCmd<string>('retry_agent_run', instruction ? { runId, instruction } : { runId });
}

export async function getPlanTasks(sessionId: string) {
  return invokeCmd<PlanTaskRecord[]>('get_plan_tasks', { sessionId });
}

export async function createPlanTask(
  sessionId: string,
  title: string,
  options?: { parentId?: string | null; runId?: string | null; source?: string; changeReason?: string | null },
) {
  return invokeCmd<PlanTaskRecord>('create_plan_task', {
    sessionId,
    title,
    parentId: options?.parentId,
    runId: options?.runId,
    source: options?.source,
    changeReason: options?.changeReason,
  });
}

export async function updatePlanTask(
  patch: PlanTaskPatch,
  options?: { changeReason?: string | null },
) {
  return invokeCmd<PlanTaskRecord>('update_plan_task', {
    payload: patch,
    changeReason: options?.changeReason,
  });
}

export async function updatePlanTaskStatus(
  id: string,
  status: string,
  runId?: string | null,
  options?: { changeReason?: string | null },
) {
  return invokeCmd<PlanTaskRecord>('update_plan_task_status', {
    id,
    status,
    runId,
    changeReason: options?.changeReason,
  });
}

export async function archivePlanTask(id: string, options?: { changeReason?: string | null }) {
  return invokeCmd<void>('archive_plan_task', { id, changeReason: options?.changeReason });
}

export async function setActivePlanTask(
  sessionId: string,
  taskId: string | null,
  options?: { changeReason?: string | null },
) {
  return invokeCmd<void>('set_active_plan_task', {
    sessionId,
    taskId,
    changeReason: options?.changeReason,
  });
}

export async function waivePlanTask(id: string, reason?: string | null) {
  return invokeCmd<PlanTaskRecord>('waive_plan_task', { id, reason });
}

export async function getPlanChangeEvents(sessionId: string, runId?: string | null, limit = 80) {
  return invokeCmd<PlanChangeRecord[]>('get_plan_change_events', { sessionId, runId, limit });
}

export async function scanPlanTaskRunIntegrity() {
  return invokeCmd<PlanTaskRunIntegrityReport>('scan_plan_task_run_integrity');
}

export async function repairPlanTaskRunIntegrity(changeReason?: string | null) {
  return invokeCmd<PlanTaskRunIntegrityReport>('repair_plan_task_run_integrity', { changeReason });
}

export interface ProviderCapabilitiesRow {
  providerId: string;
  model: string;
  vision: boolean;
  toolCalls: boolean;
  jsonMode: boolean;
  maxContext: number;
  source: string;
  updatedAt: number;
}

export async function listProviderCapabilities() {
  return invokeCmd<ProviderCapabilitiesRow[]>('list_provider_capabilities', {});
}

export async function resolveProviderCapabilities(providerId: string, model: string) {
  return invokeCmd<ProviderCapabilitiesRow>('resolve_provider_capabilities', {
    providerId,
    model,
  });
}

export interface EndpointProbe {
  reachable: boolean;
  models: string[];
  queriedModelFound: boolean;
  detail: string;
}

export interface ProbeProviderResult {
  probe: EndpointProbe;
  capabilities: ProviderCapabilitiesRow;
  error: string | null;
}

export async function probeProviderCapabilities(providerId: string, model: string) {
  return invokeCmd<ProbeProviderResult>('probe_provider_capabilities', {
    providerId,
    model,
  });
}

export async function resetProviderCapabilities(providerId: string) {
  return invokeCmd<number>('reset_provider_capabilities', { providerId });
}

export interface CapabilityAuditRow {
  id: number;
  providerId: string;
  model: string;
  field: string;
  oldValue: string | null;
  newValue: string | null;
  sourceBefore: string | null;
  sourceAfter: string;
  changedAt: number;
}

export async function listCapabilityAudit(
  providerId?: string,
  model?: string,
  limit = 100,
) {
  return invokeCmd<CapabilityAuditRow[]>('list_capability_audit', {
    providerId,
    model,
    limit,
  });
}

export async function overrideProviderCapabilities(args: {
  providerId: string;
  model: string;
  vision: boolean;
  toolCalls: boolean;
  jsonMode: boolean;
  maxContext: number;
}) {
  return invokeCmd<ProviderCapabilitiesRow>('override_provider_capabilities', args);
}

export async function getArtifacts(sessionId?: string, limit = 80) {
  return invokeCmd<ArtifactRecord[]>('get_artifacts', { sessionId, limit });
}

export async function getBrowserAuditEvents(limit = 80) {
  return invokeCmd<Record<string, any>[]>('get_browser_audit_events', { limit });
}

/* ============================================================================
 * @deprecated 灵动岛（Island）域 — 弃用域，不进 Atlas 主 UI。
 * 浮窗/贴纸/截屏 overlay 前端组件与路由已在 UI-0.2 移除。以下 island_* wrapper
 * 与对应后端 Tauri 命令暂时保留（后端命令暂留，后续单独清理卡处理），但 Atlas
 * 新 UI 一律不接入这些 wrapper。新增功能请勿调用本段。
 * ========================================================================== */
export type IslandScreenshotMode = 'screen' | 'window' | 'area';

export interface IslandScreenshotRequest {
  mode?: IslandScreenshotMode;
  x?: number;
  y?: number;
  width?: number;
  height?: number;
}

export interface IslandScreenshotResult {
  ok: boolean;
  mode: IslandScreenshotMode;
  tempPath: string;
  mime: string;
  width: number;
  height: number;
  x: number;
  y: number;
  capturedAt: number;
  source: string;
  dataUrl: string;
  size: number;
}

export interface IslandOcrResult {
  ok: boolean;
  available: boolean;
  source: string;
  text: string;
  lines?: string[];
  language?: string;
  confidence?: number | null;
  confidenceAvailable?: boolean;
  qualityWarnings?: string[];
  warning?: string | null;
  reason?: string | null;
  capturedAt: number;
}

export interface IslandShortcutCheckInput {
  id: string;
  label: string;
  accelerator: string;
}

export interface IslandShortcutCheckItem {
  id: string;
  label: string;
  accelerator: string;
  ok: boolean;
  status: 'available' | 'conflict' | 'invalid' | 'registered_by_aura' | 'unconfigured' | 'unavailable' | 'failed';
  reason?: string | null;
}

export interface IslandShortcutConflictResult {
  ok: boolean;
  status: 'available' | 'conflict' | 'invalid' | 'registered_by_aura' | 'unconfigured' | 'unavailable' | 'failed';
  items: IslandShortcutCheckItem[];
  checkedAt: number;
}

export interface IslandSavePathPermissionResult {
  ok: boolean;
  status: 'unconfigured' | 'writable' | 'missing' | 'not_directory' | 'denied' | 'failed';
  directory: string;
  reason?: string | null;
  checkedAt: number;
}

export interface IslandContextExportPayload {
  directory: string;
  fileName: string;
  imagePath?: string;
  dataUrl?: string;
  text?: string;
}

export interface IslandContextExportResult {
  ok: boolean;
  status: 'saved';
  path: string;
  fileName: string;
  bytes: number;
  savedAt: number;
}

export async function islandGetSettings(): Promise<IslandSettings> {
  return invokeCmd<IslandSettings>('island_get_settings');
}

export async function islandSaveSettings(settings: IslandSettings): Promise<IslandSettings> {
  return invokeCmd<IslandSettings>('island_save_settings', { settings });
}

export async function islandShowMainWindow(): Promise<void> {
  return invokeCmd<void>('island_show_main_window');
}

export async function islandGetWindowContext(): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('island_get_window_context');
}

export async function islandReadClipboard(): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('island_read_clipboard');
}

export async function islandCaptureScreenshot(payload?: IslandScreenshotRequest): Promise<IslandScreenshotResult> {
  return invokeCmd<IslandScreenshotResult>('island_capture_screenshot', payload ? { payload } : undefined);
}

export async function islandRunOcr(imagePath: string, languages?: string): Promise<IslandOcrResult> {
  return invokeCmd<IslandOcrResult>('island_run_ocr', { payload: { imagePath, languages } });
}

export async function islandCheckShortcutConflicts(shortcuts: IslandShortcutCheckInput[]): Promise<IslandShortcutConflictResult> {
  return invokeCmd<IslandShortcutConflictResult>('island_check_shortcut_conflicts', { payload: { shortcuts } });
}

export async function islandCheckSavePathPermission(directory: string): Promise<IslandSavePathPermissionResult> {
  return invokeCmd<IslandSavePathPermissionResult>('island_check_save_path_permission', { payload: { directory } });
}

export async function islandSaveContextExport(payload: IslandContextExportPayload): Promise<IslandContextExportResult> {
  return invokeCmd<IslandContextExportResult>('island_save_context_export', { payload });
}

export async function islandGetMediaStatus(): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('island_get_media_status');
}

export async function islandControlMedia(action: 'playPause' | 'next' | 'previous'): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('island_control_media', { payload: { action } });
}

export async function islandGetSystemStatus(requestKind: 'auto_glance' | 'manual_button' = 'manual_button'): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('island_get_system_status', { requestKind });
}

export interface IslandKeyboardSmokeProofPayload {
  action: 'expand' | 'collapse';
  detail: number;
  key?: string;
  controlAriaLabel?: string;
  controlDataset?: Record<string, string>;
  activeElement?: string;
  expandedBefore: boolean;
  expandedAfter: boolean;
  source?: string;
}

export async function islandWriteKeyboardSmokeProof(payload: IslandKeyboardSmokeProofPayload): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('island_write_keyboard_smoke_proof', { payload });
}

export async function writeSettingsSmokeProof(payload: { section: string; title: string; source?: string }): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('write_settings_smoke_proof', { payload });
}

export async function writeSettingsPersistenceSmokeProof(payload: Record<string, unknown>): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('write_settings_persistence_smoke_proof', { payload });
}

export async function writeSettingsDomainSmokeProof(payload: Record<string, unknown>): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('write_settings_domain_smoke_proof', { payload });
}

export async function islandLogContextSent(packageId: string, source: string): Promise<void> {
  return invokeCmd<void>('island_log_context_sent', { packageId, source });
}

export async function islandLogContextImported(
  packageId: string,
  source: string,
  proof?: { attachmentCount?: number; imageAttached?: boolean; tempImageImported?: boolean; textLength?: number },
): Promise<void> {
  return invokeCmd<void>('island_log_context_imported', { packageId, source, proof });
}

export async function islandCleanupTempFile(path: string): Promise<void> {
  return invokeCmd<void>('island_cleanup_temp_file', { path });
}

export async function islandReadTempImage(path: string): Promise<{ dataUrl: string; mime: string; size: number; tempPath: string }> {
  return invokeCmd<{ dataUrl: string; mime: string; size: number; tempPath: string }>('island_read_temp_image', { path });
}

export interface FeishuReceivedEvent {
  id: string;
  eventId: string;
  eventType: string;
  chatId?: string | null;
  senderId?: string | null;
  messageType?: string | null;
  text?: string | null;
  raw: Record<string, unknown>;
  receivedAt: number;
}

export interface FeishuCallbackStatus {
  running: boolean;
  localUrl?: string | null;
  publicUrl?: string | null;
  receivedCount: number;
  lastEventAt?: number | null;
  message: string;
  receiveReady?: boolean;
  requirements?: FeishuReceiveRequirement[];
}

export interface FeishuReceiveRequirement {
  id: string;
  label: string;
  status: string;
  detail: string;
}

export interface FeishuTunnelStatus {
  running: boolean;
  provider: string;
  publicUrl?: string | null;
  callbackUrl?: string | null;
  localUrl: string;
  startedAt?: number | null;
  message: string;
}

export interface FeishuSetupLinks {
  appId: string;
  appHomeUrl: string;
  permissionUrl: string;
  eventUrl: string;
  requiredChatScopes: string[];
  requiredMessageScopes: string[];
  requiredEventType: string;
}

export interface FeishuCallbackResult {
  statusCode: number;
  body: Record<string, unknown>;
  event?: FeishuReceivedEvent | null;
}

export async function startFeishuCallbackServer(port?: number) {
  return invokeCmd<FeishuCallbackStatus>('start_feishu_callback_server', port ? { port } : undefined);
}

export async function stopFeishuCallbackServer() {
  return invokeCmd<FeishuCallbackStatus>('stop_feishu_callback_server');
}

export async function getFeishuCallbackStatus() {
  return invokeCmd<FeishuCallbackStatus>('get_feishu_callback_status');
}

export async function setFeishuPublicUrl(publicUrl?: string | null) {
  return invokeCmd<FeishuCallbackStatus>('set_feishu_public_url', { publicUrl });
}

export async function getFeishuSetupLinks() {
  return invokeCmd<FeishuSetupLinks>('get_feishu_setup_links');
}

export async function startFeishuPublicTunnel() {
  return invokeCmd<FeishuTunnelStatus>('start_feishu_public_tunnel');
}

export async function stopFeishuPublicTunnel() {
  return invokeCmd<FeishuTunnelStatus>('stop_feishu_public_tunnel');
}

export async function getFeishuTunnelStatus() {
  return invokeCmd<FeishuTunnelStatus>('get_feishu_tunnel_status');
}

export async function getFeishuReceivedEvents(limit = 80) {
  return invokeCmd<FeishuReceivedEvent[]>('get_feishu_received_events', { limit });
}

export async function ingestFeishuEventPayload(payload: Record<string, unknown>) {
  return invokeCmd<FeishuCallbackResult>('ingest_feishu_event_payload', { payload });
}

export interface McpServerConfig {
  id: string;
  name: string;
  transport: 'stdio' | 'http_sse' | string;
  command?: string | null;
  args: string[];
  url?: string | null;
  env?: McpKeyValue[];
  passEnv?: string[];
  cwd?: string | null;
  headers?: McpKeyValue[];
  authType?: string | null;
  authToken?: string | null;
  enabled: boolean;
  risk: 'safe' | 'sensitive' | 'destructive' | string;
  trusted: boolean;
  trustedAt?: number | null;
  createdAt: number;
  updatedAt: number;
  lastStatus?: string | null;
  lastError?: string | null;
}

export interface McpKeyValue {
  key: string;
  value: string;
}

export interface McpToolInfo {
  name: string;
  title: string;
  description: string;
  readOnly: boolean;
  risk: string;
  requiresConfirmation: boolean;
}

export interface McpServerStatus {
  server: McpServerConfig;
  status: string;
  message: string;
  tools: McpToolInfo[];
  resources: string[];
  prompts: string[];
}

export interface McpAuditEvent {
  id: string;
  serverId: string;
  serverName: string;
  toolName: string;
  action: string;
  risk: string;
  confirmed: boolean;
  status: string;
  reason: string;
  inputSummary: string;
  createdAt: number;
}

export interface McpServerInput {
  id?: string;
  name: string;
  transport: 'stdio' | 'http_sse' | string;
  command?: string | null;
  args?: string[];
  url?: string | null;
  env?: McpKeyValue[];
  passEnv?: string[];
  cwd?: string | null;
  headers?: McpKeyValue[];
  authType?: string | null;
  authToken?: string | null;
  enabled: boolean;
  risk: 'safe' | 'sensitive' | 'destructive' | string;
}

export async function getMcpServers() {
  return invokeCmd<McpServerConfig[]>('get_mcp_servers');
}

export async function saveMcpServer(payload: McpServerInput) {
  return invokeCmd<McpServerConfig>('save_mcp_server', { payload });
}

export async function deleteMcpServer(id: string) {
  return invokeCmd<void>('delete_mcp_server', { id });
}

export async function setMcpServerTrust(id: string, trusted: boolean) {
  return invokeCmd<McpServerConfig>('set_mcp_server_trust', { id, trusted });
}

export async function testMcpServer(id: string) {
  return invokeCmd<McpServerStatus>('test_mcp_server', { id });
}

export async function invokeMcpTool(serverId: string, toolName: string, args: Record<string, unknown> = {}, confirmed = false) {
  return invokeCmd<Record<string, unknown>>('invoke_mcp_tool', {
    serverId,
    toolName,
    arguments: args,
    confirmed,
  });
}

export async function getMcpAuditEvents(limit = 80) {
  return invokeCmd<McpAuditEvent[]>('get_mcp_audit_events', { limit });
}

export async function getConfig() {
  return invokeCmd<any>('get_config');
}

export async function saveConfig(cfg: Record<string, unknown>) {
  return invokeCmd<void>('save_config', { config: normalizeConfigForRust(cfg) });
}

export async function getBackendStatus() {
  return invokeCmd<{
    provider: string;
    provider_supported: boolean;
    api_key_configured: boolean;
    autostart_available: boolean;
    notifications_available: boolean;
    updater_available: boolean;
    local_scan_available: boolean;
    tts_available: boolean;
  }>('get_backend_status');
}

export interface WebSearchItem {
  title: string;
  url: string;
  snippet: string;
}

export interface WebSearchResponse {
  query: string;
  provider: string;
  searchUrl: string;
  results: WebSearchItem[];
  warning?: string | null;
}

export interface OpenWebSearchResult {
  query: string;
  engine: string;
  url: string;
  opened: boolean;
}

export interface WebPageExtract {
  url: string;
  finalUrl: string;
  title: string;
  description: string;
  text: string;
  chars: number;
  truncated: boolean;
}

export interface GithubTrendingRepository {
  rank: number;
  owner: string;
  name: string;
  fullName: string;
  url: string;
  description: string;
  language?: string | null;
  stars: string;
  forks: string;
  starsToday: string;
}

export interface GithubTrendingResponse {
  sourceUrl: string;
  since: string;
  repositories: GithubTrendingRepository[];
  count: number;
}

export async function searchWeb(query: string, limit = 5) {
  return invokeCmd<WebSearchResponse>('search_web', { query, limit });
}

export async function openExternalWebSearch(query: string, engine = 'duckduckgo') {
  return invokeCmd<OpenWebSearchResult>('open_external_web_search', { query, engine });
}

export async function fetchWebPage(url: string, maxChars = 12000) {
  return invokeCmd<WebPageExtract>('fetch_web_page', { url, maxChars });
}

export async function getGithubTrending(language?: string, since = 'daily', limit = 12) {
  return invokeCmd<GithubTrendingResponse>('get_github_trending', { language, since, limit });
}

export interface SessionRecord {
  id: string;
  title: string;
  project_id?: string | null;
  title_is_manual?: boolean;
  pinned?: boolean;
  archived_at?: number | null;
  created_at: number;
  updated_at: number;
  last_active_at?: number;
}

export interface ProjectRecord {
  id: string;
  title: string;
  root_path?: string | null;
  kind: string;
  pinned?: boolean;
  archived_at?: number | null;
  created_at: number;
  updated_at: number;
  last_active_at: number;
}

export interface ConversationHistoryReport {
  rangeLabel: string;
  sessionCount: number;
  messageCount: number;
  userMessageCount: number;
  assistantMessageCount: number;
  report: string;
  generatedAt: number;
}

export interface CodeReviewCommandRules {
  path: string;
  content: string;
  warnings: string[];
}

export interface InitAgentRulesResult {
  scope: 'global' | 'project';
  path: string;
  created: boolean;
}

export interface GlobalAgentRulesRecord {
  path: string;
  content: string;
}

export interface MessageRecord {
  id: string;
  session_id: string;
  role: string;
  content: string;
  created_at: number;
  metadata: unknown;
}

export interface SaveMessagePayload {
  id?: string;
  role: string;
  content: string;
  created_at?: number;
  metadata?: unknown;
}

export interface MemoryRecord {
  id: string;
  text: string;
  source: string;
  enabled: boolean;
  quality?: string;
  confidence?: number;
  lastUsedAt?: number | null;
  useCount?: number;
  createdAt?: number;
  updatedAt?: number;
  created_at?: number;
  updated_at?: number;
}

export interface ContextUsageSnapshot {
  sessionId: string | null;
  usedTokens: number;
  limitTokens: number;
  ratio: number;
  source: 'estimated' | 'exact' | string;
  compressionState: 'draft' | 'ok' | 'near_limit' | 'compressed' | string;
  summaryIncluded?: boolean;
  messageCount?: number;
  updatedAt: number;
}

export interface ProfileRecord {
  id: string;
  profile: any;
  updated_at: number;
}

export interface PersonalityProgressRecord {
  id: string;
  progress: any;
  updated_at: number;
}

export interface SessionSummary {
  sessionId: string;
  summary: string;
  sourceMessageCount: number;
  updatedAt: number;
}

export interface AgentToolMetadata {
  name: string;
  description: string;
  labelZh?: string;
  descriptionZh?: string;
  capabilityLabelsZh?: string[];
  safetyLabelZh?: string;
  capabilities: string[];
  safety_level: 'safe' | 'sensitive' | 'destructive' | string;
  mutates_state: boolean;
  requires_confirmation: boolean;
}

export interface AgentSkillMetadata {
  name: string;
  description: string;
  labelZh?: string;
  descriptionZh?: string;
  triggers: string[];
  allowedTools: string[];
  tags: string[];
  source: string;
  enabled?: boolean;
  builtIn?: boolean;
  pendingReview?: boolean;
  sourceKind?: string;
  path?: string | null;
  origin?: string | null;
  loadError?: string | null;
  stateKey?: string;
}

export interface AgentProfileMetadata {
  name: string;
  description: string;
  model?: string;
  tools?: string[];
  source: string;
  sourceKind: string;
}

export interface AgentSkillInput {
  name: string;
  labelZh?: string;
  description: string;
  descriptionZh?: string;
  triggers?: string[];
  allowedTools?: string[];
  body: string;
  overwrite?: boolean;
}

export interface PersonalityQuestion {
  id: string;
  dimension: string;
  text: string;
  options: Array<{ label: string; value: number }>;
}

export interface PersonalityQuestionSet {
  mode: string;
  title: string;
  estimatedMinutes: number;
  questions: PersonalityQuestion[];
}

export interface ActivityEvent {
  id: string;
  date: string;
  kind: 'agent' | 'memory' | 'profile' | 'system' | string;
  title: string;
  detail: string;
  metadata: any;
  createdAt: number;
}

export interface LogActivityEventPayload {
  date?: string;
  kind: ActivityEvent['kind'];
  title: string;
  detail?: string;
  metadata?: any;
}

export interface FileWritePreview {
  id: string;
  targetPath: string;
  operation: 'create' | 'overwrite' | string;
  contentSize: number;
  preview: string;
  existingPreview?: string | null;
  diff?: string | null;
  reason: string;
  createdAt: number;
}

export interface PendingCommand {
  id: string;
  command: string;
  cwd: string;
  reason: string;
  shell: string;
}

export interface CommandExecutionResult {
  command: string;
  cwd: string;
  exitCode?: number | null;
  stdout: string;
  stderr: string;
  timedOut: boolean;
}

export async function initLocalDb() {
  return invokeCmd<string>('init_local_db');
}

export async function getSessions() {
  return invokeCmd<SessionRecord[]>('get_sessions');
}

export async function listProjects() {
  return invokeCmd<ProjectRecord[]>('list_projects');
}

export async function upsertProject(title: string | null, rootPath: string) {
  return invokeCmd<ProjectRecord>('upsert_project', { title, rootPath });
}

export async function createProjectFolder(title: string) {
  return invokeCmd<ProjectRecord>('create_project_folder', { title });
}

export async function renameProject(id: string, title: string) {
  return invokeCmd<ProjectRecord>('rename_project', { id, title });
}

export async function setProjectPinned(id: string, pinned: boolean) {
  return invokeCmd<ProjectRecord>('set_project_pinned', { id, pinned });
}

export async function archiveProject(id: string) {
  return invokeCmd<ProjectRecord>('archive_project', { id });
}

export async function deleteProject(id: string) {
  return invokeCmd<ProjectRecord[]>('delete_project', { id });
}

export async function openProjectInExplorer(id: string) {
  return invokeCmd<void>('open_project_in_explorer', { id });
}

export async function generateHistoryReport(range?: string) {
  return invokeCmd<ConversationHistoryReport>('generate_history_report', range ? { range } : undefined);
}

export async function getArchivedSessions() {
  return invokeCmd<SessionRecord[]>('get_archived_sessions');
}

export async function searchSessions(query: string, options?: { archived?: boolean }) {
  return invokeCmd<SessionRecord[]>('search_sessions', { query, archived: Boolean(options?.archived) });
}

export async function createSession(title: string, projectId?: string | null) {
  return invokeCmd<SessionRecord>('create_session', projectId ? { title, projectId } : { title });
}

export async function renameSession(id: string, title: string) {
  return invokeCmd<SessionRecord>('rename_session', { id, title });
}

export async function deleteSession(id: string) {
  return invokeCmd<SessionRecord[]>('delete_session', { id });
}

export async function archiveSession(id: string) {
  return invokeCmd<SessionRecord[]>('archive_session', { id });
}

export async function restoreSession(id: string) {
  return invokeCmd<SessionRecord>('restore_session', { id });
}

export async function setSessionPinned(id: string, pinned: boolean) {
  return invokeCmd<SessionRecord>('set_session_pinned', { id, pinned });
}

export async function getMessages(sessionId: string) {
  return invokeCmd<MessageRecord[]>('get_messages', { sessionId });
}

export async function saveMessage(sessionId: string, message: SaveMessagePayload) {
  return invokeCmd<MessageRecord>('save_message', { sessionId, message });
}

export async function clearSessionContext(sessionId: string) {
  return invokeCmd<number>('clear_session_context', { sessionId });
}

export async function initAgentRules(projectRoot?: string | null) {
  return invokeCmd<InitAgentRulesResult>(
    'init_agent_rules',
    projectRoot ? { projectRoot } : undefined,
  );
}

export async function readGlobalAgentRules() {
  return invokeCmd<GlobalAgentRulesRecord>('read_global_agent_rules');
}

export async function saveGlobalAgentRules(content: string) {
  return invokeCmd<GlobalAgentRulesRecord>('save_global_agent_rules', { content });
}

export async function getCodeReviewCommandRules(projectRoot?: string | null) {
  return invokeCmd<CodeReviewCommandRules>(
    'get_code_review_command_rules',
    projectRoot ? { projectRoot } : undefined,
  );
}

export async function getMemories() {
  return invokeCmd<MemoryRecord[]>('get_memories');
}

export async function addMemory(text: string, source = 'manual') {
  return invokeCmd<MemoryRecord>('add_memory', { text, source });
}

export async function updateMemory(id: string, text?: string, enabled?: boolean) {
  return invokeCmd<MemoryRecord>('update_memory', { id, text, enabled });
}

export async function deleteMemory(id: string) {
  return invokeCmd<void>('delete_memory', { id });
}

export async function clearMemories() {
  return invokeCmd<void>('clear_memories');
}

export async function getProfile() {
  return invokeCmd<ProfileRecord>('get_profile');
}

export async function saveProfile(profile: any) {
  return invokeCmd<ProfileRecord>('save_profile', { profile });
}

export async function startPersonalityTest() {
  return invokeCmd<PersonalityProgressRecord>('start_personality_test');
}

export async function getPersonalityQuestions(mode: string) {
  return invokeCmd<PersonalityQuestionSet>('get_personality_questions', { mode });
}

export async function getPersonalityProgress() {
  return invokeCmd<PersonalityProgressRecord>('get_personality_progress');
}

export async function savePersonalityProgress(progress: any) {
  return invokeCmd<PersonalityProgressRecord>('save_personality_progress', { progress });
}

export async function completePersonalityTest(answers: any[]) {
  return invokeCmd<ProfileRecord>('complete_personality_test', { answers });
}

export async function prepareFileWrite(path: string, content: string, reason?: string) {
  return invokeCmd<FileWritePreview>('prepare_file_write', { path, content, reason });
}

export async function confirmFileWrite(id: string, editedText?: string, options?: { sessionId?: string; runId?: string }) {
  return invokeCmd<FileWritePreview>('confirm_file_write', {
    id,
    editedText,
    sessionId: options?.sessionId,
    runId: options?.runId,
  });
}

export async function rejectFileWrite(id: string) {
  return invokeCmd<void>('reject_file_write', { id });
}

export async function runApprovedCommand(id: string) {
  return invokeCmd<CommandExecutionResult>('run_approved_command', { id });
}

export async function rejectPendingCommand(id: string) {
  return invokeCmd<void>('reject_pending_command', { id });
}

export async function getAppState<T = unknown>(key: string) {
  return invokeCmd<T | null>('get_app_state', { key });
}

export async function setAppState(key: string, value: unknown) {
  return invokeCmd<void>('set_app_state', { key, value });
}

export async function writeAgentWorkbenchSmokeProof(payload: Record<string, unknown>): Promise<Record<string, any>> {
  return invokeCmd<Record<string, any>>('write_agent_workbench_smoke_proof', { payload });
}

export async function getPersonalityOnboardingState<T = unknown>() {
  return invokeCmd<T | null>('get_personality_onboarding_state');
}

export async function savePersonalityOnboardingState(value: unknown) {
  return invokeCmd<void>('save_personality_onboarding_state', { value });
}

export interface ResetLocalDataOptions {
  sessions?: boolean;
  memories?: boolean;
  profile?: boolean;
  appState?: boolean;
}

export interface ResetLocalDataSummary {
  resetScopes: string[];
  preservedConfig: boolean;
  replacementSession?: SessionRecord | null;
  updatedAt: number;
}

export interface LocalDbHealth {
  ok: boolean;
  dbPath: string;
  sessions: number;
  messages: number;
  memories: number;
  activityEvents: number;
  appState: number;
  checkedAt: number;
}

export async function exportLocalData() {
  return invokeCmd<string>('export_local_data');
}

export async function resetLocalData(options: ResetLocalDataOptions) {
  return invokeCmd<ResetLocalDataSummary>('reset_local_data', { options });
}

export async function getLocalDbHealth() {
  return invokeCmd<LocalDbHealth>('get_local_db_health');
}

export async function logActivityEvent(payload: LogActivityEventPayload) {
  return invokeCmd<ActivityEvent>('log_activity_event', { payload });
}

export async function logAuraActivity(kind: LogActivityEventPayload['kind'], title: string, detail = '', metadata: any = {}) {
  if (!isTauriRuntime()) return null;
  try {
    return await logActivityEvent({ kind, title, detail, metadata });
  } catch {
    return null;
  }
}

export async function getRecentActivityEvents(date?: string, limit = 20) {
  return invokeCmd<ActivityEvent[]>('get_recent_activity_events', { date, limit });
}

function asRecord(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
}

function normalizeConfigForRust(cfg: Record<string, unknown>) {
  const ui = asRecord(cfg.ui);
  const llm = asRecord(cfg.llm);
  const connections = Array.isArray(llm.connections) ? (llm.connections as Record<string, unknown>[]) : [];
  const defaultConnectionId = String(llm.default_connection_id ?? llm.defaultConnectionId ?? '');
  const activeConnection = connections.find((connection) => String(connection.id ?? '') === defaultConnectionId) ?? connections[0] ?? {};
  const provider = String(
    cfg.provider ??
      cfg.provider_id ??
      cfg.providerId ??
      activeConnection.provider_id ??
      activeConnection.providerId ??
      llm.default_provider ??
      llm.defaultProvider ??
      'openai'
  );
  return {
    connection_id: cfg.connection_id ?? cfg.connectionId ?? activeConnection.id,
    provider,
    provider_id: cfg.provider_id ?? cfg.providerId ?? activeConnection.provider_id ?? activeConnection.providerId,
    route_id: cfg.route_id ?? cfg.routeId,
    connection_name: cfg.connection_name ?? cfg.connectionName ?? activeConnection.name,
    protocol: cfg.protocol ?? activeConnection.protocol,
    api_url: String(cfg.api_url ?? cfg.apiUrl ?? activeConnection.base_url ?? activeConnection.baseUrl ?? ''),
    api_key: cfg.api_key ?? cfg.apiKey,
    clear_api_key: Boolean(cfg.clear_api_key ?? cfg.clearApiKey ?? false),
    model_name: String(cfg.model_name ?? cfg.modelName ?? activeConnection.model ?? activeConnection.model_name ?? ''),
    auth_header: cfg.auth_header ?? cfg.authHeader ?? activeConnection.auth_header ?? activeConnection.authHeader,
    theme: String(cfg.theme ?? ui.theme ?? 'dark'),
    sound_enabled: Boolean(cfg.sound_enabled ?? cfg.soundEnabled ?? ui.sound_enabled ?? ui.soundEnabled ?? false),
  };
}

function normalizeModelSettingsPayload(payload: ModelSettingsPayload) {
  const provider = String(payload.provider || 'openai');
  return {
    connection_id: payload.connectionId,
    provider,
    provider_id: payload.providerId,
    route_id: payload.routeId,
    protocol: payload.protocol,
    api_url: payload.apiUrl || '',
    api_key: payload.apiKey,
    clear_api_key: payload.clearApiKey === true,
    model_name: payload.modelName || '',
    auth_header: payload.authHeader,
  };
}
