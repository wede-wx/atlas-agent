export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
export type JsonRecord = Record<string, JsonValue>;
export type UnknownRecord = Record<string, unknown>;

export interface SessionRecord {
  id: string;
  title: string;
  project_id: string | null;
  title_is_manual: boolean;
  pinned: boolean;
  archived_at: number | null;
  created_at: number;
  updated_at: number;
  last_active_at: number;
}

export interface ProjectRecord {
  id: string;
  title: string;
  root_path: string;
  pinned?: boolean;
  archived_at?: number | null;
  created_at?: number;
  updated_at?: number;
}

export interface MessageRecord {
  id: string;
  session_id: string;
  role: string;
  content: string;
  created_at: number;
  metadata: unknown;
}

export interface ToolCall {
  id: string;
  name: string;
  arguments: unknown;
}

export type AgentRunEvent =
  | { type: "Started"; run: unknown }
  | { type: "Iteration"; run_id: string; iteration: number }
  | { type: "ToolResult"; run_id: string; result: unknown }
  | { type: "GuidanceMerged"; run_id: string; count: number }
  | { type: "GuidanceQueued"; run_id: string; count: number }
  | { type: "Finished"; run_id: string }
  | { type: "Blocked"; run_id: string; status: string; footer: string }
  | { type: "Paused"; run_id: string }
  | { type: "Resumed"; run_id: string }
  | { type: "Cancelled"; run_id: string }
  | { type: "Failed"; run_id: string; error: string; retryable: boolean };

export type AgentEvent =
  | { type: "Thinking"; content: string }
  | { type: "ResponseStarted"; message_id: string }
  | { type: "ResponseDelta"; message_id: string; content: string }
  | { type: "ResponseCompleted"; message_id: string; content: string }
  | { type: "ResponseFallbackStarted"; message_id: string; reason: string }
  | { type: "Response"; message_id: string; content: string }
  | { type: "OperationStarted"; operation_id: string; tool_name: string; label: string; detail: string | null; target: string | null; command: string | null }
  | { type: "OperationOutput"; operation_id: string; stream: string; content: string }
  | { type: "OperationFinished"; operation_id: string; status: string; summary: string }
  | { type: "OperationFailed"; operation_id: string; summary: string }
  | { type: "OperationPreparing"; label: string; detail: string | null; tool_name: string | null; bytes: number | null }
  | { type: "OperationProgress"; label: string; detail: string | null; tool_name: string | null; bytes: number | null }
  | { type: "ToolCall"; tool_call: ToolCall }
  | { type: "ToolResult"; result: string }
  | { type: "SubAgentStarted"; subagent_id: string; name: string; description: string; task: string }
  | { type: "SubAgentFinished"; subagent_id: string; name: string; summary: string }
  | { type: "SubAgentFailed"; subagent_id: string; name: string; error: string }
  | { type: "ToolVisibilityDecision"; tools_enabled: boolean; intent: string; advertised_tools: string[]; hidden_reason: string | null }
  | { type: "ModelToolParseDiagnostic"; returned_kind: string; parsed: boolean; reason: string | null }
  | { type: "UnknownToolRequested"; requested: string; nearest: string | null }
  | { type: "ToolNormalizationApplied"; original_name: string; normalized_name: string; argument_changes: string[] }
  | { type: "FinalAudit"; run_id: string; audit: unknown }
  | { type: "RunEvent"; event: AgentRunEvent };

export interface AgentEventEnvelope {
  sessionId: string;
  runId: string;
  event: AgentEvent;
}

export interface AgentRunRecord extends UnknownRecord {
  id: string;
  session_id?: string | null;
  status?: string;
  created_at?: number;
  updated_at?: number;
}

export interface RunTimelineEntry {
  kind: string;
  id: string;
  at: number;
  finished_at: number | null;
  seq: number;
  label: string;
  status: string | null;
  detail: unknown;
}

export interface RunTimeline {
  run_id: string;
  run: AgentRunRecord | null;
  total: number;
  offset: number;
  limit: number;
  entries: RunTimelineEntry[];
}

export interface RunAuditFeed extends UnknownRecord {
  run_id?: string;
  total?: number;
  offset?: number;
  limit?: number;
  entries?: UnknownRecord[];
}

export interface RunProgressSummary extends UnknownRecord {
  run_id?: string;
  status?: string;
  latest_message?: string;
  completed_steps?: number;
  failed_steps?: number;
}

export interface PermissionDecisionRecord {
  id: string;
  session_id: string | null;
  run_id: string;
  iteration: number;
  tool_call_id: string;
  subject: string;
  action: string;
  risk: string;
  mode: string;
  decision: string;
  reason: string;
  decided_by: string;
  created_at: number;
}

export interface AgentGraphNode extends UnknownRecord {
  id?: string;
  node_id?: string;
  label?: string;
  name?: string;
  kind?: string;
  status?: string;
}

export interface AgentGraphEdge extends UnknownRecord {
  id?: string;
  from?: string;
  to?: string;
  source?: string;
  target?: string;
  status?: string;
}

export interface AgentGraphSnapshot extends UnknownRecord {
  graph_run_id?: string;
  graphRunId?: string;
  run_id?: string;
  nodes?: AgentGraphNode[];
  edges?: AgentGraphEdge[];
  checkpoints?: UnknownRecord[];
}

export interface WorkflowTraceReport extends UnknownRecord {
  graph_run_id?: string;
  graphRunId?: string;
  traces?: UnknownRecord[];
  entries?: UnknownRecord[];
}

export interface KnowledgeRecallRequest {
  query: string;
  scope?: string | null;
  limit?: number | null;
}

export interface RetrievalHitRecord {
  item_id: string;
  scope: string;
  source: string;
  trust: string;
  title: string;
  snippet: string;
  score: number;
  confidence: number;
  reason: string;
  embedding_ref: string | null;
  created_at: number;
}

export interface RetrievalContext {
  hits: RetrievalHitRecord[];
  system_note: string | null;
}

export interface AddKnowledgeItemPayload {
  id?: string | null;
  scope: string;
  source: string;
  trust: string;
  title: string;
  text: string;
  confidence?: number | null;
  expires_at?: number | null;
  embedding_ref?: string | null;
}

export interface KnowledgeItemRecord extends AddKnowledgeItemPayload {
  id: string;
  enabled: boolean;
  confidence: number;
  created_at: number;
  updated_at: number;
  deleted_at: number | null;
}

export interface PluginPackageRecord extends UnknownRecord {
  id: string;
  name: string;
  version: string;
  source: string;
  description: string;
  trusted: boolean;
  enabled: boolean;
  risk: string;
  installed_at: number;
  updated_at: number;
}

export interface EvalSuite extends UnknownRecord {
  id: string;
  name: string;
  kind: string;
  description: string;
  cases: Array<{ id: string; title: string; category: string; tags: string[] }>;
}

export interface McpServerConfig extends UnknownRecord {
  id: string;
  name: string;
  transport: string;
  enabled: boolean;
  trusted: boolean;
  risk: string;
  last_status: string | null;
  last_error: string | null;
}

export interface UiThemePreference { mode: "dark" | "light" | "system" }
export interface UiNotificationPreference { runCompleted: boolean; blockedGate: boolean; permissionNeeded: boolean; sound: boolean }
export interface UiGeneralPreference { defaultAgentMode: "chat" | "agent"; autoCreateSession: boolean; openDrawerOnRun: boolean }
export interface UiLayoutPreference { sidebarCollapsed: boolean; rightDrawerOpen: boolean; rightDrawerTab: "contract" | "timeline" | "graph" }

export interface UiPreferences {
  theme: UiThemePreference;
  notifications: UiNotificationPreference;
  general: UiGeneralPreference;
  layout: UiLayoutPreference;
}

export type AppView = "chat" | "knowledge" | "plugins" | "evals" | "settings";

export function asRecord(value: unknown): UnknownRecord {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as UnknownRecord) : {};
}

export function textOf(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}
