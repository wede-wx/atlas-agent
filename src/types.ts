// Types mirrored from the Rust backend (src-tauri/src/agent/types.rs and
// src-tauri/src/storage/mod.rs). The backend structs carry no serde
// rename_all, so JSON fields are snake_case and enum tags are PascalCase.
// Keep this file in sync with the Rust side — it is the typed contract a UI
// builds against. See docs/COMMAND_BRIDGE.md.

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

export interface MessageRecord {
  id: string;
  session_id: string;
  role: string; // "user" | "assistant" | "system"
  content: string;
  created_at: number;
  metadata: unknown;
}

export interface ToolCall {
  id: string;
  name: string;
  arguments: unknown;
}

// ---- Streamed run events (AgentRunEvent, tag = "type") ----
export type AgentRunEvent =
  | { type: "Started"; run: unknown }
  | { type: "Iteration"; run_id: string; iteration: number }
  | { type: "ToolResult"; run_id: string; result: unknown }
  | { type: "GuidanceMerged"; run_id: string; count: number }
  | { type: "GuidanceQueued"; run_id: string; count: number }
  | { type: "Finished"; run_id: string }
  // final_audit decided the response cannot be marked complete — the soul of
  // the project. status is "blocked" | "unverified"; footer is the audit text.
  | { type: "Blocked"; run_id: string; status: string; footer: string }
  | { type: "Paused"; run_id: string }
  | { type: "Resumed"; run_id: string }
  | { type: "Cancelled"; run_id: string }
  | { type: "Failed"; run_id: string; error: string; retryable: boolean };

// ---- Streamed agent events (AgentEvent, tag = "type") ----
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

// Envelope emitted on the "agent-event" channel.
// Confirmed shape from agent_chat_v2; if your event forwarder wraps differently,
// adjust ENVELOPE handling in bridge.ts (documented there).
export interface AgentEventEnvelope {
  sessionId: string;
  runId: string;
  event: AgentEvent;
}
