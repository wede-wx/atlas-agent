import type {
  AddKnowledgeItemPayload,
  AgentEventEnvelope,
  AgentGraphSnapshot,
  AgentRunRecord,
  EvalSuite,
  JsonValue,
  KnowledgeItemRecord,
  KnowledgeRecallRequest,
  McpServerConfig,
  MessageRecord,
  PermissionDecisionRecord,
  PluginPackageRecord,
  ProjectRecord,
  RetrievalContext,
  RunProgressSummary,
  RunTimeline,
  SessionRecord,
  UiPreferences,
  UnknownRecord,
  WorkflowTraceReport,
} from "./types";

type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
type ListenFn = (event: string, cb: (e: { payload: unknown }) => void) => Promise<() => void>;

let invokeRef: InvokeFn | null = null;
let listenRef: ListenFn | null = null;

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function api(): Promise<{ invoke: InvokeFn; listen: ListenFn }> {
  if (invokeRef && listenRef) return { invoke: invokeRef, listen: listenRef };
  const core = await import("@tauri-apps/api/core");
  const event = await import("@tauri-apps/api/event");
  invokeRef = core.invoke as unknown as InvokeFn;
  listenRef = event.listen as unknown as ListenFn;
  return { invoke: invokeRef, listen: listenRef };
}

async function invokeCommand<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await api();
  return invoke<T>(cmd, args);
}

export async function minimizeWindow(): Promise<void> {
  if (!isTauri()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().minimize();
}

export async function toggleMaximizeWindow(): Promise<void> {
  if (!isTauri()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().toggleMaximize();
}

export async function closeWindow(): Promise<void> {
  if (!isTauri()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().close();
}

export async function getSessions(): Promise<SessionRecord[]> {
  return invokeCommand<SessionRecord[]>("get_sessions");
}

export async function getArchivedSessions(): Promise<SessionRecord[]> {
  return invokeCommand<SessionRecord[]>("get_archived_sessions");
}

export async function searchSessions(query: string, archived?: boolean): Promise<SessionRecord[]> {
  return invokeCommand<SessionRecord[]>("search_sessions", { query, archived: archived ?? null });
}

export async function createSession(title: string, projectId?: string | null): Promise<SessionRecord> {
  return invokeCommand<SessionRecord>("create_session", { title, projectId: projectId ?? null });
}

export async function renameSession(id: string, title: string): Promise<SessionRecord> {
  return invokeCommand<SessionRecord>("rename_session", { id, title });
}

export async function deleteSession(id: string): Promise<SessionRecord[]> {
  return invokeCommand<SessionRecord[]>("delete_session", { id });
}

export async function archiveSession(id: string): Promise<SessionRecord[]> {
  return invokeCommand<SessionRecord[]>("archive_session", { id });
}

export async function setSessionPinned(id: string, pinned: boolean): Promise<SessionRecord> {
  return invokeCommand<SessionRecord>("set_session_pinned", { id, pinned });
}

export async function listProjects(): Promise<ProjectRecord[]> {
  return invokeCommand<ProjectRecord[]>("list_projects");
}

export async function getMessages(sessionId: string): Promise<MessageRecord[]> {
  return invokeCommand<MessageRecord[]>("get_messages", { sessionId });
}

export async function agentChat(opts: {
  sessionId: string;
  message: string;
  displayMessage?: string;
  mode?: string;
}): Promise<string> {
  return invokeCommand<string>("agent_chat_v2", {
    sessionId: opts.sessionId,
    message: opts.message,
    displayMessage: opts.displayMessage ?? null,
    mode: opts.mode ?? "chat",
    attachments: null,
  });
}

export async function cancelAgentChat(sessionId?: string | null): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("cancel_agent_chat", { sessionId: sessionId ?? null });
}

export async function pauseAgentChat(sessionId?: string | null): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("pause_agent_chat", { sessionId: sessionId ?? null });
}

export async function resumeAgentChat(sessionId?: string | null): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("resume_agent_chat", { sessionId: sessionId ?? null });
}

export async function onAgentEvent(handler: (env: AgentEventEnvelope) => void): Promise<() => void> {
  const { listen } = await api();
  return listen("agent-event", ({ payload }) => {
    const maybe = payload as Record<string, unknown>;
    if (maybe && typeof maybe === "object" && "event" in maybe) {
      handler(maybe as unknown as AgentEventEnvelope);
    } else if (maybe && typeof maybe === "object" && "type" in maybe) {
      handler({ sessionId: "", runId: String(maybe.runId ?? maybe.run_id ?? ""), event: maybe as never });
    }
  });
}

export async function getAgentRuns(sessionId?: string | null, limit = 20): Promise<AgentRunRecord[]> {
  return invokeCommand<AgentRunRecord[]>("get_agent_runs", { sessionId: sessionId ?? null, limit });
}

export async function getAgentRunTimeline(runId: string, limit = 80, offset = 0): Promise<RunTimeline> {
  return invokeCommand<RunTimeline>("get_agent_run_timeline", { runId, limit, offset });
}

export async function getAgentRunProgress(runId: string): Promise<RunProgressSummary> {
  return invokeCommand<RunProgressSummary>("get_agent_run_progress", { runId });
}

export async function getAgentRunDiff(runId: string, limit = 40): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("get_agent_run_diff", { runId, limit });
}

export async function getAgentRunTerminal(runId: string, limit = 50, offset = 0): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("get_agent_run_terminal", { runId, limit, offset });
}

export async function getAgentRunAudit(runId: string, limit = 50, offset = 0): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("get_agent_run_audit", { runId, limit, offset });
}

export async function getAgentPermissionDecisions(runId: string, limit = 50): Promise<PermissionDecisionRecord[]> {
  return invokeCommand<PermissionDecisionRecord[]>("get_agent_permission_decisions", { runId, limit });
}

export async function getAgentGraphSnapshot(graphRunId: string): Promise<AgentGraphSnapshot> {
  return invokeCommand<AgentGraphSnapshot>("get_agent_graph_snapshot", { graphRunId });
}

export async function getAgentGraphNodeTraces(graphRunId: string): Promise<WorkflowTraceReport> {
  return invokeCommand<WorkflowTraceReport>("get_agent_graph_node_traces", { graphRunId });
}

export async function resolvePermissionConfirmation(record: PermissionDecisionRecord, approved: boolean, impact?: string): Promise<PermissionDecisionRecord> {
  return invokeCommand<PermissionDecisionRecord>("resolve_permission_confirmation", {
    runId: record.run_id,
    iteration: record.iteration,
    toolCallId: record.tool_call_id,
    subject: record.subject,
    action: record.action,
    risk: record.risk,
    mode: record.mode,
    approved,
    impact: impact ?? null,
    sessionId: record.session_id,
  });
}

export async function getConfig(): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("get_config");
}

export async function saveConfig(config: UnknownRecord): Promise<void> {
  await invokeCommand<void>("save_config", { config });
}

export async function deleteModelConnection(connectionId: string): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("delete_model_connection", { connectionId });
}

export async function revealModelConnectionKey(connectionId: string): Promise<string> {
  return invokeCommand<string>("reveal_model_connection_key", { connectionId });
}

export async function checkModelSettings(payload: UnknownRecord): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("check_model_settings", { payload });
}

export async function listModels(payload: UnknownRecord): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("list_models", { payload });
}

export async function getBackendStatus(): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("get_backend_status");
}

export async function getAppState<T>(key: string): Promise<T | null> {
  return invokeCommand<T | null>("get_app_state", { key });
}

export async function setAppState(key: string, value: JsonValue): Promise<void> {
  await invokeCommand<void>("set_app_state", { key, value });
}

export async function getUiPreferences(defaults: UiPreferences): Promise<UiPreferences> {
  const [theme, notifications, general, layout] = await Promise.all([
    getAppState<UiPreferences["theme"]>("ui.theme"),
    getAppState<UiPreferences["notifications"]>("ui.notifications"),
    getAppState<UiPreferences["general"]>("ui.general"),
    getAppState<UiPreferences["layout"]>("ui.layout"),
  ]);
  return {
    theme: theme ?? defaults.theme,
    notifications: notifications ?? defaults.notifications,
    general: general ?? defaults.general,
    layout: layout ?? defaults.layout,
  };
}

export async function saveUiPreference<K extends keyof UiPreferences>(key: K, value: UiPreferences[K]): Promise<void> {
  const storageKey = `ui.${key}`;
  await setAppState(storageKey, value as unknown as JsonValue);
}

export async function readGlobalAgentRules(): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("read_global_agent_rules");
}

export async function saveGlobalAgentRules(content: string): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("save_global_agent_rules", { content });
}

export async function initAgentRules(projectRoot?: string | null): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("init_agent_rules", { projectRoot: projectRoot ?? null });
}

export async function getMcpServers(): Promise<McpServerConfig[]> {
  return invokeCommand<McpServerConfig[]>("get_mcp_servers");
}

export async function testMcpServer(id: string): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("test_mcp_server", { id });
}

export async function setMcpServerTrust(id: string, trusted: boolean): Promise<McpServerConfig> {
  return invokeCommand<McpServerConfig>("set_mcp_server_trust", { id, trusted });
}

export async function searchKnowledge(request: KnowledgeRecallRequest): Promise<RetrievalContext> {
  return invokeCommand<RetrievalContext>("search_knowledge", { request });
}

export async function addKnowledgeItem(payload: AddKnowledgeItemPayload): Promise<KnowledgeItemRecord> {
  return invokeCommand<KnowledgeItemRecord>("add_knowledge_item", { payload });
}

export async function deleteKnowledgeItem(id: string): Promise<KnowledgeItemRecord> {
  return invokeCommand<KnowledgeItemRecord>("delete_knowledge_item", { id });
}

export async function listPluginPackages(): Promise<PluginPackageRecord[]> {
  return invokeCommand<PluginPackageRecord[]>("list_plugin_packages");
}

export async function setPluginPackageEnabled(id: string, enabled: boolean): Promise<PluginPackageRecord> {
  return invokeCommand<PluginPackageRecord>("set_plugin_package_enabled", { id, enabled });
}

export async function getPluginCapabilityEvents(pluginId?: string | null, limit = 50): Promise<UnknownRecord[]> {
  return invokeCommand<UnknownRecord[]>("get_plugin_capability_events", { pluginId: pluginId ?? null, limit });
}

export async function getAgentEvalSuites(): Promise<EvalSuite[]> {
  return invokeCommand<EvalSuite[]>("get_agent_eval_suites");
}

export async function runAgentEvalSuiteVerifiers(suiteId: string, caseIds?: string[] | null, cwd?: string | null, claimedComplete = false): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("run_agent_eval_suite_verifiers", {
    suiteId,
    caseIds: caseIds ?? null,
    cwd: cwd ?? null,
    claimedComplete,
  });
}

export async function scoreAgentEvalSuite(suiteId: string, outcomes: UnknownRecord[]): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("score_agent_eval_suite", { suiteId, outcomes });
}

export async function getMemories(): Promise<UnknownRecord[]> {
  return invokeCommand<UnknownRecord[]>("get_memories");
}

export async function addMemory(text: string, source?: string | null): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("add_memory", { text, source: source ?? null });
}

export async function exportLocalData(): Promise<string> {
  return invokeCommand<string>("export_local_data");
}

export async function resetLocalData(options: { sessions?: boolean; memories?: boolean; profile?: boolean; app_state?: boolean }): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("reset_local_data", { options });
}

export async function getLocalDbHealth(): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("get_local_db_health");
}

export async function resolveAtlasDeviation(opts: {
  sessionId: string;
  itemId: string;
  target: string;
  approved: boolean;
  runId?: string | null;
  toolName?: string | null;
}): Promise<UnknownRecord> {
  return invokeCommand<UnknownRecord>("resolve_atlas_deviation", {
    sessionId: opts.sessionId,
    itemId: opts.itemId,
    target: opts.target,
    approved: opts.approved,
    runId: opts.runId ?? null,
    toolName: opts.toolName ?? null,
  });
}

