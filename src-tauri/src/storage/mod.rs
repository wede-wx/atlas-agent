use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct LocalDb {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("validation error: {0}")]
    Validation(String),
}

pub type StorageResult<T> = Result<T, StorageError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub title_is_manual: bool,
    pub pinned: bool,
    pub archived_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_active_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub id: String,
    pub title: String,
    pub root_path: Option<String>,
    pub kind: String,
    pub pinned: bool,
    pub archived_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_active_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: i64,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveMessagePayload {
    pub id: Option<String>,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub text: String,
    pub source: String,
    pub enabled: bool,
    pub quality: String,
    pub confidence: f64,
    pub last_used_at: Option<i64>,
    pub use_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileRecord {
    pub id: String,
    pub profile: serde_json::Value,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalityProgressRecord {
    pub id: String,
    pub progress: serde_json::Value,
    pub updated_at: i64,
}

const AGENT_TOOL_AUDIT_RETENTION_ROWS: i64 = 2000;
/// P0-4: cap on retained permission-decision rows (mirrors the tool-audit cap).
const PERMISSION_DECISION_RETENTION_ROWS: i64 = 2000;

/// P2-5: long-term memory decay policy. The decay clock is keyed on `updated_at`
/// (last time a memory was created / edited / re-confirmed), NOT `last_used_at`
/// — being injected into context every turn is "use", not "reinforcement", so it
/// must not reset the decay clock. A memory idle (un-reconfirmed) longer than the
/// window loses `MEMORY_DECAY_STEP` confidence each maintenance pass until it
/// drops below `MEMORY_PURGE_FLOOR`, at which point it is soft-disabled.
const MEMORY_DECAY_IDLE_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const MEMORY_DECAY_STEP: f64 = 0.1;
const MEMORY_PURGE_FLOOR: f64 = 0.2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub summary: String,
    pub source_message_count: usize,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationHistoryReport {
    pub range_label: String,
    pub session_count: usize,
    pub message_count: usize,
    pub user_message_count: usize,
    pub assistant_message_count: usize,
    pub report: String,
    pub generated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub id: String,
    pub date: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub metadata: serde_json::Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolAuditRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub run_id: String,
    pub iteration: i64,
    pub tool_call_id: String,
    pub tool_name: String,
    pub permission_mode: String,
    pub policy: String,
    pub status: String,
    pub reason: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogAgentToolAuditPayload {
    pub session_id: Option<String>,
    pub run_id: String,
    pub iteration: usize,
    pub tool_call_id: String,
    pub tool_name: String,
    pub permission_mode: String,
    pub policy: String,
    pub status: String,
    pub reason: String,
}

/// P0-4: an independent, queryable permission decision (doc §8.3
/// `PermissionDecision`). Distinct from `AgentToolAuditRecord` — that table logs
/// every tool event (incl. execution outcomes); this one is the decision ledger
/// that answers "谁批 / 为什么 / 何时" for any guarded action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDecisionRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub run_id: String,
    pub iteration: i64,
    pub tool_call_id: String,
    /// file | command | git | network | mcp | other
    pub subject: String,
    pub action: String,
    /// safe | sensitive | destructive
    pub risk: String,
    pub mode: String,
    /// allowed | needs_confirm | denied
    pub decision: String,
    pub reason: String,
    /// gate | policy | skill | hard_rule
    pub decided_by: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPermissionDecisionPayload {
    pub session_id: Option<String>,
    pub run_id: String,
    pub iteration: usize,
    pub tool_call_id: String,
    pub subject: String,
    pub action: String,
    pub risk: String,
    pub mode: String,
    pub decision: String,
    pub reason: String,
    pub decided_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsageRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub run_id: String,
    pub iteration: i64,
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub source: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogModelUsagePayload {
    pub session_id: Option<String>,
    pub run_id: String,
    pub iteration: usize,
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsageSummary {
    pub events: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub recent: Vec<ModelUsageRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCommandRecord {
    pub id: String,
    pub command: String,
    pub cwd: String,
    pub reason: String,
    pub shell: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub status: String,
    pub permission_mode: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunStepRecord {
    pub id: String,
    pub run_id: String,
    pub step_index: i64,
    pub step_type: String,
    pub status: String,
    pub summary: String,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

/// P1-3: one entry in a run's unified replay timeline. `detail` carries the full
/// original record (step / tool / usage / verify / permission), so replay shows
/// the real input/output/result — never a fabricated terminal/diff/audit state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTimelineEntry {
    /// "step" | "tool" | "usage" | "verify" | "permission"
    pub kind: String,
    /// source row id (stable identity for the UI)
    pub id: String,
    /// ordering timestamp in ms: step/tool/usage `created_at`, verify `started_at`.
    pub at: i64,
    /// end timestamp where the source has one (step/verify `finished_at`).
    pub finished_at: Option<i64>,
    /// `step_index` for steps, `iteration` for tool/usage; -1 when not applicable.
    pub seq: i64,
    /// short label: step_type / tool_name / "provider/model" / verify kind.
    pub label: String,
    /// status where the source has one; None for usage.
    pub status: Option<String>,
    /// the full original record, serialized — the real event payload.
    pub detail: serde_json::Value,
}

/// P1-3: a paginated slice of a run's replay timeline. `total` is the run's full
/// event count, so the caller can page through the entire run in order (full
/// replay) rather than only reading the latest N events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTimeline {
    pub run_id: String,
    pub run: Option<AgentRunRecord>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
    pub entries: Vec<RunTimelineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAgentStepRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub step_index: i64,
    pub action: String,
    pub target: Option<String>,
    pub status: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub screenshot_path: Option<String>,
    pub dom_summary: serde_json::Value,
    pub action_json: serde_json::Value,
    pub result_json: serde_json::Value,
    pub fingerprint: String,
    pub judge: serde_json::Value,
    pub loop_detected: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordBrowserAgentStepPayload {
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub action: String,
    pub target: Option<String>,
    pub status: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub screenshot_path: Option<String>,
    pub dom_summary: serde_json::Value,
    pub action_json: serde_json::Value,
    pub result_json: serde_json::Value,
    pub fingerprint: String,
    pub judge: serde_json::Value,
    pub loop_detected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphRunRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub source_run_id: Option<String>,
    pub goal: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentGraphRunPayload {
    pub id: Option<String>,
    pub session_id: Option<String>,
    pub source_run_id: Option<String>,
    pub goal: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphNodeRecord {
    pub id: String,
    pub graph_run_id: String,
    pub node_key: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub attempt: i64,
    pub max_attempts: i64,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentGraphNodePayload {
    pub id: Option<String>,
    pub graph_run_id: String,
    pub node_key: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub max_attempts: Option<i64>,
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphEdgeRecord {
    pub id: String,
    pub graph_run_id: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub condition: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentGraphEdgePayload {
    pub id: Option<String>,
    pub graph_run_id: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphCheckpointRecord {
    pub id: String,
    pub graph_run_id: String,
    pub node_id: Option<String>,
    pub state: serde_json::Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphSnapshot {
    pub run: AgentGraphRunRecord,
    pub nodes: Vec<AgentGraphNodeRecord>,
    pub edges: Vec<AgentGraphEdgeRecord>,
    pub checkpoints: Vec<AgentGraphCheckpointRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamRunRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub source_run_id: Option<String>,
    pub goal: String,
    pub status: String,
    pub max_rounds: i64,
    pub termination_reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamRunPayload {
    pub id: Option<String>,
    pub session_id: Option<String>,
    pub source_run_id: Option<String>,
    pub goal: String,
    pub max_rounds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamParticipantRecord {
    pub id: String,
    pub team_run_id: String,
    pub name: String,
    pub role: String,
    pub model: Option<String>,
    pub tool_scope: serde_json::Value,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamParticipantPayload {
    pub id: Option<String>,
    pub team_run_id: String,
    pub name: String,
    pub role: String,
    pub model: Option<String>,
    #[serde(default)]
    pub tool_scope: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMessageRecord {
    pub id: String,
    pub team_run_id: String,
    pub participant_id: Option<String>,
    pub role: String,
    pub message_type: String,
    pub content: String,
    pub metadata: serde_json::Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendTeamMessagePayload {
    pub id: Option<String>,
    pub team_run_id: String,
    pub participant_id: Option<String>,
    pub role: String,
    pub message_type: String,
    pub content: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffRequestRecord {
    pub id: String,
    pub team_run_id: String,
    pub from_participant_id: Option<String>,
    pub to_participant_id: String,
    pub status: String,
    pub reason: String,
    pub contract: serde_json::Value,
    pub result: serde_json::Value,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateHandoffRequestPayload {
    pub id: Option<String>,
    pub team_run_id: String,
    pub from_participant_id: Option<String>,
    pub to_participant_id: String,
    pub reason: String,
    #[serde(default)]
    pub contract: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamRunSnapshot {
    pub run: TeamRunRecord,
    pub participants: Vec<TeamParticipantRecord>,
    pub messages: Vec<TeamMessageRecord>,
    pub handoffs: Vec<HandoffRequestRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeItemRecord {
    pub id: String,
    pub scope: String,
    pub source: String,
    pub trust: String,
    pub title: String,
    pub text: String,
    pub enabled: bool,
    pub confidence: f64,
    pub expires_at: Option<i64>,
    pub embedding_ref: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddKnowledgeItemPayload {
    pub id: Option<String>,
    pub scope: String,
    pub source: String,
    pub trust: String,
    pub title: String,
    pub text: String,
    pub confidence: Option<f64>,
    pub expires_at: Option<i64>,
    pub embedding_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalHitRecord {
    pub item_id: String,
    pub scope: String,
    pub source: String,
    pub trust: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
    pub confidence: f64,
    pub reason: String,
    pub embedding_ref: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRelevanceFeedbackReport {
    pub reinforced_item_ids: Vec<String>,
    pub decayed_item_ids: Vec<String>,
    pub soft_deleted_item_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLifecycleRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub root_path: String,
    pub status: String,
    pub setup_status: String,
    pub sandbox_backend: String,
    pub sandbox_status: String,
    pub fallback_reason: Option<String>,
    pub setup_script: Option<String>,
    pub audit: serde_json::Value,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceLifecyclePayload {
    pub id: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub root_path: String,
    pub sandbox_backend: Option<String>,
    pub fallback_reason: Option<String>,
    pub setup_script: Option<String>,
    #[serde(default)]
    pub audit: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSetupEventRecord {
    pub id: String,
    pub workspace_id: String,
    pub stage: String,
    pub status: String,
    pub command: Option<String>,
    pub exit_code: Option<i64>,
    pub output_tail: String,
    pub reason: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordWorkspaceSetupEventPayload {
    pub workspace_id: String,
    pub stage: String,
    pub status: String,
    pub command: Option<String>,
    pub exit_code: Option<i64>,
    pub output_tail: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLifecycleSnapshot {
    pub workspace: WorkspaceLifecycleRecord,
    pub events: Vec<WorkspaceSetupEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistEvalRunPayload {
    pub id: String,
    pub suite_id: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub status: String,
    pub cwd: String,
    pub passed: bool,
    pub started_at: i64,
    pub finished_at: i64,
    pub report: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistEvalCaseResultPayload {
    pub eval_run_id: String,
    pub case_id: String,
    pub status: String,
    pub passed: bool,
    pub verified: bool,
    pub false_completion: bool,
    pub blocked: bool,
    pub artifact_path: Option<String>,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistEvalCommandResultPayload {
    pub eval_run_id: String,
    pub case_id: String,
    pub command: String,
    pub cwd: String,
    pub required: bool,
    pub status: String,
    pub exit_code: Option<i64>,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub started_at: i64,
    pub finished_at: i64,
    pub duration_ms: i64,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalRunStorageRecord {
    pub id: String,
    pub suite_id: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub status: String,
    pub cwd: String,
    pub passed: bool,
    pub started_at: i64,
    pub finished_at: i64,
    pub report: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTaskRecord {
    pub id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub parent_id: Option<String>,
    pub title: String,
    pub status: String,
    pub position: i64,
    pub source: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    #[serde(default)]
    pub acceptance_criteria: serde_json::Value,
    #[serde(default)]
    pub verify: serde_json::Value,
    #[serde(default)]
    pub evidence: serde_json::Value,
    #[serde(default = "default_evidence_status")]
    pub evidence_status: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTaskRunIntegrityIssue {
    pub task_id: String,
    pub session_id: String,
    pub run_id: String,
    pub issue: String,
    #[serde(default)]
    pub run_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTaskRunIntegrityReport {
    pub checked_at: i64,
    pub scanned_tasks: i64,
    pub issue_count: usize,
    pub repaired_count: usize,
    pub repair_applied: bool,
    pub issues: Vec<PlanTaskRunIntegrityIssue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTaskPatch {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub clear_parent_id: bool,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub clear_run_id: bool,
    #[serde(default)]
    pub acceptance_criteria: Option<serde_json::Value>,
    #[serde(default)]
    pub clear_acceptance_criteria: bool,
    #[serde(default)]
    pub verify: Option<serde_json::Value>,
    #[serde(default)]
    pub clear_verify: bool,
}

fn default_evidence_status() -> String {
    "none".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPlanRecord {
    pub id: String,
    pub run_id: Option<String>,
    pub session_id: String,
    pub goal: String,
    /// P3-4: the observable outcome that proves the goal is met (a result, not
    /// an action). `None` for legacy plans created before Goal 对象正式化.
    #[serde(default)]
    pub observable_outcome: Option<String>,
    /// P3-4: explicit non-goals (Array<String>); `null`/`[]` when unset.
    #[serde(default)]
    pub non_goals: serde_json::Value,
    #[serde(default)]
    pub acceptance_criteria: serde_json::Value,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanChangeRecord {
    pub id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub actor: String,
    pub action: String,
    pub subject_type: String,
    pub subject_id: String,
    pub reason: String,
    #[serde(default)]
    pub before: Value,
    #[serde(default)]
    pub after: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPackageRecord {
    pub id: String,
    pub name: String,
    pub version: String,
    pub source: String,
    pub description: String,
    pub trusted: bool,
    pub enabled: bool,
    pub risk: String,
    #[serde(default)]
    pub permissions: Value,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(default)]
    pub manifest: Value,
    pub installed_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCapabilityEventRecord {
    pub id: String,
    pub plugin_id: String,
    pub capability_id: String,
    pub action: String,
    pub status: String,
    pub risk: String,
    pub reason: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub output: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct LogPluginCapabilityEventPayload {
    pub plugin_id: String,
    pub capability_id: String,
    pub action: String,
    pub status: String,
    pub risk: String,
    pub reason: String,
    pub input: Value,
    pub output: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskVerificationRecord {
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
pub struct FileCheckpointRecord {
    pub id: String,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub path: String,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub after_content: Option<String>,
    pub before_content: Option<String>,
    pub before_blob_path: Option<String>,
    pub before_size: i64,
    pub created_at: i64,
    pub restored_at: Option<i64>,
}

/// Persisted provider capability row (M5). Matches the `provider_capabilities`
/// table 1:1; in-memory analogue lives in `agent::capabilities::ProviderCapabilities`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilitiesRow {
    pub provider_id: String,
    pub model: String,
    pub vision: bool,
    pub tool_calls: bool,
    pub json_mode: bool,
    pub max_context: u32,
    pub source: String,
    pub updated_at: i64,
}

/// T30: one audit log entry recording a change to provider_capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityAuditRow {
    pub id: i64,
    pub provider_id: String,
    pub model: String,
    pub field: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub source_before: Option<String>,
    pub source_after: String,
    pub changed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub path: Option<String>,
    pub operation: String,
    pub status: String,
    pub summary: String,
    pub metadata: serde_json::Value,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordArtifactPayload {
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub path: Option<String>,
    pub operation: String,
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogActivityEventPayload {
    #[serde(default)]
    pub date: Option<String>,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResetLocalDataOptions {
    #[serde(default)]
    pub sessions: bool,
    #[serde(default)]
    pub memories: bool,
    #[serde(default)]
    pub profile: bool,
    #[serde(default)]
    pub app_state: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetLocalDataSummary {
    pub reset_scopes: Vec<String>,
    pub preserved_config: bool,
    pub replacement_session: Option<SessionRecord>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateRecord {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDataExport {
    pub schema_version: i64,
    pub exported_at: i64,
    pub db_path: String,
    pub sessions: Vec<SessionRecord>,
    pub messages: Vec<MessageRecord>,
    pub projects: Vec<ProjectRecord>,
    pub agent_runs: Vec<AgentRunRecord>,
    pub agent_run_steps: Vec<AgentRunStepRecord>,
    pub agent_tool_audit_events: Vec<AgentToolAuditRecord>,
    pub model_usage_events: Vec<ModelUsageRecord>,
    pub plan_tasks: Vec<PlanTaskRecord>,
    /// P3-5 (schema v6): auditable plan mutation log. Older exports omit this.
    #[serde(default)]
    pub plan_change_events: Vec<PlanChangeRecord>,
    /// P3-6 (schema v7): installed plugin capability packages and audit events.
    #[serde(default)]
    pub plugin_packages: Vec<PluginPackageRecord>,
    #[serde(default)]
    pub plugin_capability_events: Vec<PluginCapabilityEventRecord>,
    pub artifacts: Vec<ArtifactRecord>,
    pub memories: Vec<MemoryRecord>,
    pub profile: ProfileRecord,
    pub personality_progress: PersonalityProgressRecord,
    pub app_state: Vec<AppStateRecord>,
    pub activity_events: Vec<ActivityEvent>,
    /// T31 (schema v5): provider capability matrix + audit log. Older v4/v3
    /// bundles omit these fields; serde default lets import fall back gracefully.
    #[serde(default)]
    pub provider_capabilities: Vec<ProviderCapabilitiesRow>,
    #[serde(default)]
    pub capability_audit: Vec<CapabilityAuditRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDbHealth {
    pub ok: bool,
    pub db_path: String,
    pub sessions: i64,
    pub messages: i64,
    pub memories: i64,
    pub activity_events: i64,
    pub app_state: i64,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileWritePreview {
    pub id: String,
    pub target_path: String,
    pub operation: String,
    pub content_size: usize,
    pub preview: String,
    pub existing_preview: Option<String>,
    pub diff: Option<String>,
    pub reason: String,
    pub created_at: i64,
}

impl LocalDb {
    pub fn open_default() -> StorageResult<Self> {
        Self::open(atlas_home()?.join("atlas.db"))
    }

    pub fn open(path: PathBuf) -> StorageResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&path)?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
        };
        db.init()?;
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn init(&self) -> StorageResult<()> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                root_path TEXT UNIQUE,
                kind TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                last_active_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                text TEXT NOT NULL,
                source TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS profiles (
                id TEXT PRIMARY KEY,
                profile_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS personality_progress (
                id TEXT PRIMARY KEY,
                progress_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS app_state (
                key TEXT PRIMARY KEY,
                value_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS activity_events (
                id TEXT PRIMARY KEY,
                date TEXT NOT NULL,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                detail TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_activity_events_date_created
                ON activity_events(date, created_at DESC);

            CREATE TABLE IF NOT EXISTS agent_tool_audit_events (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                run_id TEXT NOT NULL,
                iteration INTEGER NOT NULL,
                tool_call_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                permission_mode TEXT NOT NULL,
                policy TEXT NOT NULL,
                status TEXT NOT NULL,
                reason TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_agent_tool_audit_created
                ON agent_tool_audit_events(created_at DESC);

            CREATE INDEX IF NOT EXISTS idx_agent_tool_audit_session_created
                ON agent_tool_audit_events(session_id, created_at DESC);

            CREATE INDEX IF NOT EXISTS idx_agent_tool_audit_run_created
                ON agent_tool_audit_events(run_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS permission_decisions (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                run_id TEXT NOT NULL,
                iteration INTEGER NOT NULL,
                tool_call_id TEXT NOT NULL,
                subject TEXT NOT NULL,
                action TEXT NOT NULL,
                risk TEXT NOT NULL,
                mode TEXT NOT NULL,
                decision TEXT NOT NULL,
                reason TEXT NOT NULL,
                decided_by TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_permission_decisions_run_created
                ON permission_decisions(run_id, created_at ASC);

            CREATE INDEX IF NOT EXISTS idx_permission_decisions_created
                ON permission_decisions(created_at DESC);

            CREATE TABLE IF NOT EXISTS model_usage_events (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                run_id TEXT NOT NULL,
                iteration INTEGER NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                source TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_model_usage_created
                ON model_usage_events(created_at DESC);

            CREATE INDEX IF NOT EXISTS idx_model_usage_run_created
                ON model_usage_events(run_id, created_at DESC);

            CREATE INDEX IF NOT EXISTS idx_model_usage_session_created
                ON model_usage_events(session_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS agent_runs (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                status TEXT NOT NULL,
                permission_mode TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                finished_at INTEGER,
                error TEXT,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_agent_runs_session_created
                ON agent_runs(session_id, created_at DESC);

            CREATE INDEX IF NOT EXISTS idx_agent_runs_status_updated
                ON agent_runs(status, updated_at DESC);

            CREATE TABLE IF NOT EXISTS agent_run_steps (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                step_index INTEGER NOT NULL,
                step_type TEXT NOT NULL,
                status TEXT NOT NULL,
                summary TEXT NOT NULL,
                input_json TEXT NOT NULL DEFAULT '{}',
                output_json TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL,
                finished_at INTEGER,
                FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_agent_run_steps_run_index
                ON agent_run_steps(run_id, step_index);

            CREATE TABLE IF NOT EXISTS pending_file_writes (
                id TEXT PRIMARY KEY,
                target_path TEXT NOT NULL,
                content TEXT NOT NULL,
                reason TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                resolved_at INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_pending_file_writes_status_created
                ON pending_file_writes(status, created_at DESC);

            CREATE TABLE IF NOT EXISTS pending_commands (
                id TEXT PRIMARY KEY,
                command TEXT NOT NULL,
                cwd TEXT NOT NULL,
                reason TEXT NOT NULL,
                shell TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                resolved_at INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_pending_commands_status_created
                ON pending_commands(status, created_at DESC);

            CREATE TABLE IF NOT EXISTS plan_tasks (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                run_id TEXT,
                parent_id TEXT,
                title TEXT NOT NULL,
                status TEXT NOT NULL,
                position INTEGER NOT NULL DEFAULT 0,
                source TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                archived_at INTEGER,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE SET NULL,
                FOREIGN KEY(parent_id) REFERENCES plan_tasks(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_plan_tasks_session_position
                ON plan_tasks(session_id, archived_at, position, created_at);

            CREATE TABLE IF NOT EXISTS artifacts (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                run_id TEXT,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                path TEXT,
                operation TEXT NOT NULL,
                status TEXT NOT NULL,
                summary TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL,
                FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_artifacts_session_updated
                ON artifacts(session_id, updated_at DESC);

            CREATE INDEX IF NOT EXISTS idx_artifacts_run_updated
                ON artifacts(run_id, updated_at DESC);

            INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (1, strftime('%s','now') * 1000);
            "#,
        )?;
        ensure_table_column(&conn, "sessions", "project_id", "TEXT")?;
        ensure_table_column(
            &conn,
            "sessions",
            "title_is_manual",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_table_column(&conn, "sessions", "pinned", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_table_column(&conn, "sessions", "archived_at", "INTEGER")?;
        ensure_table_column(&conn, "sessions", "last_active_at", "INTEGER")?;
        ensure_table_column(&conn, "projects", "root_path", "TEXT")?;
        ensure_table_column(&conn, "projects", "kind", "TEXT NOT NULL DEFAULT 'folder'")?;
        ensure_table_column(&conn, "projects", "pinned", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_table_column(&conn, "projects", "archived_at", "INTEGER")?;
        ensure_table_column(
            &conn,
            "projects",
            "created_at",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_table_column(
            &conn,
            "projects",
            "updated_at",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_table_column(&conn, "projects", "last_active_at", "INTEGER")?;
        ensure_table_column(&conn, "messages", "metadata", "TEXT NOT NULL DEFAULT '{}'")?;
        ensure_table_column(
            &conn,
            "memories",
            "quality",
            "TEXT NOT NULL DEFAULT 'confirmed'",
        )?;
        ensure_table_column(&conn, "memories", "confidence", "REAL NOT NULL DEFAULT 1.0")?;
        ensure_table_column(&conn, "memories", "last_used_at", "INTEGER")?;
        ensure_table_column(&conn, "memories", "use_count", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_table_column(
            &conn,
            "messages",
            "created_at",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (2, strftime('%s','now') * 1000)",
            [],
        )?;
        conn.execute(
            "UPDATE sessions SET last_active_at = updated_at WHERE last_active_at IS NULL",
            [],
        )?;
        conn.execute(
            "UPDATE projects SET last_active_at = updated_at WHERE last_active_at IS NULL",
            [],
        )?;
        conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_messages_session_created
                ON messages(session_id, created_at);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_root_path
                ON projects(root_path);
            CREATE INDEX IF NOT EXISTS idx_sessions_last_active
                ON sessions(last_active_at DESC);
            CREATE INDEX IF NOT EXISTS idx_sessions_project_last_active
                ON sessions(project_id, last_active_at DESC);
            CREATE INDEX IF NOT EXISTS idx_sessions_archived_last_active
                ON sessions(archived_at, last_active_at DESC);
            CREATE INDEX IF NOT EXISTS idx_projects_last_active
                ON projects(last_active_at DESC);
            CREATE INDEX IF NOT EXISTS idx_projects_pinned_last_active
                ON projects(pinned DESC, last_active_at DESC);
            CREATE INDEX IF NOT EXISTS idx_projects_archived_last_active
                ON projects(archived_at, last_active_at DESC);
            "#,
        )?;

        ensure_table_column(&conn, "plan_tasks", "acceptance_criteria_json", "TEXT")?;
        ensure_table_column(&conn, "plan_tasks", "verify_json", "TEXT")?;
        ensure_table_column(&conn, "plan_tasks", "evidence_json", "TEXT")?;
        ensure_table_column(
            &conn,
            "plan_tasks",
            "evidence_status",
            "TEXT NOT NULL DEFAULT 'none'",
        )?;
        ensure_table_column(&conn, "plan_tasks", "active", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_table_column(&conn, "plan_tasks", "blocked_reason", "TEXT")?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS run_plans (
                id TEXT PRIMARY KEY,
                run_id TEXT,
                session_id TEXT NOT NULL,
                goal TEXT NOT NULL,
                acceptance_criteria_json TEXT,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_run_plans_session_updated
                ON run_plans(session_id, updated_at DESC);

            CREATE TABLE IF NOT EXISTS plan_change_events (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                run_id TEXT,
                actor TEXT NOT NULL,
                action TEXT NOT NULL,
                subject_type TEXT NOT NULL,
                subject_id TEXT NOT NULL,
                reason TEXT NOT NULL,
                before_json TEXT NOT NULL DEFAULT 'null',
                after_json TEXT NOT NULL DEFAULT 'null',
                created_at INTEGER NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_plan_change_events_session_created
                ON plan_change_events(session_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_plan_change_events_run_created
                ON plan_change_events(run_id, created_at ASC);

            CREATE TABLE IF NOT EXISTS plugin_packages (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                source TEXT NOT NULL,
                description TEXT NOT NULL,
                trusted INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 0,
                risk TEXT NOT NULL DEFAULT 'sensitive',
                permissions_json TEXT NOT NULL DEFAULT '[]',
                capabilities_json TEXT NOT NULL DEFAULT '[]',
                manifest_json TEXT NOT NULL DEFAULT '{}',
                installed_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_plugin_packages_enabled
                ON plugin_packages(enabled, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_plugin_packages_source
                ON plugin_packages(source, updated_at DESC);

            CREATE TABLE IF NOT EXISTS plugin_capability_events (
                id TEXT PRIMARY KEY,
                plugin_id TEXT NOT NULL,
                capability_id TEXT NOT NULL,
                action TEXT NOT NULL,
                status TEXT NOT NULL,
                risk TEXT NOT NULL,
                reason TEXT NOT NULL,
                input_json TEXT NOT NULL DEFAULT 'null',
                output_json TEXT NOT NULL DEFAULT 'null',
                created_at INTEGER NOT NULL,
                FOREIGN KEY(plugin_id) REFERENCES plugin_packages(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_plugin_capability_events_plugin_created
                ON plugin_capability_events(plugin_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS run_task_verifications (
                id TEXT PRIMARY KEY,
                run_id TEXT,
                task_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                command TEXT NOT NULL,
                exit_code INTEGER,
                status TEXT NOT NULL,
                stdout_tail TEXT NOT NULL DEFAULT '',
                stderr_tail TEXT NOT NULL DEFAULT '',
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE SET NULL,
                FOREIGN KEY(task_id) REFERENCES plan_tasks(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_task_verifications_task_started
                ON run_task_verifications(task_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_task_verifications_run_started
                ON run_task_verifications(run_id, started_at DESC);

            CREATE TABLE IF NOT EXISTS run_file_checkpoints (
                id TEXT PRIMARY KEY,
                run_id TEXT,
                task_id TEXT,
                path TEXT NOT NULL,
                before_hash TEXT,
                after_hash TEXT,
                after_content TEXT,
                before_content TEXT,
                before_blob_path TEXT,
                before_size INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                restored_at INTEGER,
                FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE SET NULL,
                FOREIGN KEY(task_id) REFERENCES plan_tasks(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_file_checkpoints_task_created
                ON run_file_checkpoints(task_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_file_checkpoints_run_created
                ON run_file_checkpoints(run_id, created_at DESC);
            "#,
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (3, strftime('%s','now') * 1000)",
            [],
        )?;

        // P3-4 Goal 对象正式化: run_plans 增 observableOutcome + nonGoals（幂等升级旧库）。
        ensure_table_column(&conn, "run_plans", "observable_outcome", "TEXT")?;
        ensure_table_column(&conn, "run_plans", "non_goals_json", "TEXT")?;
        // P4-1: store post-write text snapshots so historical run diffs never
        // read today's file contents and accidentally rewrite history.
        ensure_table_column(&conn, "run_file_checkpoints", "after_content", "TEXT")?;

        // v4 (Patch 5): provider capability matrix.
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS provider_capabilities (
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                vision INTEGER NOT NULL DEFAULT 0,
                tool_calls INTEGER NOT NULL DEFAULT 0,
                json_mode INTEGER NOT NULL DEFAULT 0,
                max_context INTEGER NOT NULL DEFAULT 8192,
                source TEXT NOT NULL DEFAULT 'builtin',
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (provider_id, model)
            );

            CREATE INDEX IF NOT EXISTS idx_provider_capabilities_provider
                ON provider_capabilities(provider_id);
            "#,
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (4, strftime('%s','now') * 1000)",
            [],
        )?;

        // v5 (Patch 7 T30): capability_audit — every change to a row in
        // provider_capabilities writes a log entry, so the lineage from
        // builtin → probed → user_override is observable.
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS capability_audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                field TEXT NOT NULL,
                old_value TEXT,
                new_value TEXT,
                source_before TEXT,
                source_after TEXT NOT NULL,
                changed_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_capability_audit_provider_model
                ON capability_audit(provider_id, model, changed_at DESC);
            "#,
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (5, strftime('%s','now') * 1000)",
            [],
        )?;

        // v6 (P3-5): plan_change_events — plan/run-plan mutations now carry
        // before/after/reason/actor records that can be queried and replayed.
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (6, strftime('%s','now') * 1000)",
            [],
        )?;

        // v7 (P3-6): plugin capability packages and plugin-specific audit.
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (7, strftime('%s','now') * 1000)",
            [],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (8, strftime('%s','now') * 1000)",
            [],
        )?;

        // v9 (OS-2/OS-3): browser-agent observation steps and durable agent graph
        // runs. Browser steps are first-class replay events; graph state is stored
        // as run/node/edge/checkpoint records so orchestration can resume from DB.
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS browser_agent_steps (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                run_id TEXT,
                step_index INTEGER NOT NULL,
                action TEXT NOT NULL,
                target TEXT,
                status TEXT NOT NULL,
                title TEXT,
                url TEXT,
                screenshot_path TEXT,
                dom_summary_json TEXT NOT NULL DEFAULT '{}',
                action_json TEXT NOT NULL DEFAULT '{}',
                result_json TEXT NOT NULL DEFAULT '{}',
                fingerprint TEXT NOT NULL,
                judge_json TEXT NOT NULL DEFAULT '{}',
                loop_detected INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL,
                FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_browser_agent_steps_run_index
                ON browser_agent_steps(run_id, step_index);
            CREATE INDEX IF NOT EXISTS idx_browser_agent_steps_session_created
                ON browser_agent_steps(session_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_browser_agent_steps_fingerprint
                ON browser_agent_steps(run_id, fingerprint, created_at DESC);

            CREATE TABLE IF NOT EXISTS agent_graph_runs (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                source_run_id TEXT,
                goal TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                finished_at INTEGER,
                error TEXT,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL,
                FOREIGN KEY(source_run_id) REFERENCES agent_runs(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_agent_graph_runs_session_created
                ON agent_graph_runs(session_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_agent_graph_runs_source_run
                ON agent_graph_runs(source_run_id);

            CREATE TABLE IF NOT EXISTS agent_graph_nodes (
                id TEXT PRIMARY KEY,
                graph_run_id TEXT NOT NULL,
                node_key TEXT NOT NULL,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                status TEXT NOT NULL,
                attempt INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 1,
                input_json TEXT NOT NULL DEFAULT '{}',
                output_json TEXT NOT NULL DEFAULT '{}',
                error TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                started_at INTEGER,
                finished_at INTEGER,
                FOREIGN KEY(graph_run_id) REFERENCES agent_graph_runs(id) ON DELETE CASCADE,
                UNIQUE(graph_run_id, node_key)
            );

            CREATE INDEX IF NOT EXISTS idx_agent_graph_nodes_run_status
                ON agent_graph_nodes(graph_run_id, status, updated_at);

            CREATE TABLE IF NOT EXISTS agent_graph_edges (
                id TEXT PRIMARY KEY,
                graph_run_id TEXT NOT NULL,
                from_node_id TEXT NOT NULL,
                to_node_id TEXT NOT NULL,
                condition TEXT,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(graph_run_id) REFERENCES agent_graph_runs(id) ON DELETE CASCADE,
                FOREIGN KEY(from_node_id) REFERENCES agent_graph_nodes(id) ON DELETE CASCADE,
                FOREIGN KEY(to_node_id) REFERENCES agent_graph_nodes(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_agent_graph_edges_run_from
                ON agent_graph_edges(graph_run_id, from_node_id);
            CREATE INDEX IF NOT EXISTS idx_agent_graph_edges_run_to
                ON agent_graph_edges(graph_run_id, to_node_id);

            CREATE TABLE IF NOT EXISTS agent_graph_checkpoints (
                id TEXT PRIMARY KEY,
                graph_run_id TEXT NOT NULL,
                node_id TEXT,
                state_json TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL,
                FOREIGN KEY(graph_run_id) REFERENCES agent_graph_runs(id) ON DELETE CASCADE,
                FOREIGN KEY(node_id) REFERENCES agent_graph_nodes(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_agent_graph_checkpoints_run_created
                ON agent_graph_checkpoints(graph_run_id, created_at ASC);
            "#,
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (9, strftime('%s','now') * 1000)",
            [],
        )?;

        // v10 (OS-4/OS-5/OS-6): durable team orchestration, source-tracked
        // knowledge retrieval, and workspace lifecycle/sandbox fallback records.
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS team_runs (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                source_run_id TEXT,
                goal TEXT NOT NULL,
                status TEXT NOT NULL,
                max_rounds INTEGER NOT NULL DEFAULT 12,
                termination_reason TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                finished_at INTEGER,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL,
                FOREIGN KEY(source_run_id) REFERENCES agent_runs(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_team_runs_session_created
                ON team_runs(session_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_team_runs_source_run
                ON team_runs(source_run_id);

            CREATE TABLE IF NOT EXISTS team_participants (
                id TEXT PRIMARY KEY,
                team_run_id TEXT NOT NULL,
                name TEXT NOT NULL,
                role TEXT NOT NULL,
                model TEXT,
                tool_scope_json TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                FOREIGN KEY(team_run_id) REFERENCES team_runs(id) ON DELETE CASCADE,
                UNIQUE(team_run_id, name)
            );

            CREATE INDEX IF NOT EXISTS idx_team_participants_run_role
                ON team_participants(team_run_id, role);

            CREATE TABLE IF NOT EXISTS team_messages (
                id TEXT PRIMARY KEY,
                team_run_id TEXT NOT NULL,
                participant_id TEXT,
                role TEXT NOT NULL,
                message_type TEXT NOT NULL,
                content TEXT NOT NULL,
                metadata_json TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL,
                FOREIGN KEY(team_run_id) REFERENCES team_runs(id) ON DELETE CASCADE,
                FOREIGN KEY(participant_id) REFERENCES team_participants(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_team_messages_run_created
                ON team_messages(team_run_id, created_at ASC);

            CREATE TABLE IF NOT EXISTS handoff_requests (
                id TEXT PRIMARY KEY,
                team_run_id TEXT NOT NULL,
                from_participant_id TEXT,
                to_participant_id TEXT NOT NULL,
                status TEXT NOT NULL,
                reason TEXT NOT NULL,
                contract_json TEXT NOT NULL DEFAULT '{}',
                result_json TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL,
                resolved_at INTEGER,
                FOREIGN KEY(team_run_id) REFERENCES team_runs(id) ON DELETE CASCADE,
                FOREIGN KEY(from_participant_id) REFERENCES team_participants(id) ON DELETE SET NULL,
                FOREIGN KEY(to_participant_id) REFERENCES team_participants(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_handoff_requests_run_status
                ON handoff_requests(team_run_id, status, created_at ASC);

            CREATE TABLE IF NOT EXISTS knowledge_items (
                id TEXT PRIMARY KEY,
                scope TEXT NOT NULL,
                source TEXT NOT NULL,
                trust TEXT NOT NULL,
                title TEXT NOT NULL,
                text TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                confidence REAL NOT NULL DEFAULT 0.7,
                expires_at INTEGER,
                embedding_ref TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deleted_at INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_knowledge_items_scope_enabled
                ON knowledge_items(scope, enabled, deleted_at, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_knowledge_items_source
                ON knowledge_items(source, updated_at DESC);

            CREATE TABLE IF NOT EXISTS workspace_lifecycle_runs (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                run_id TEXT,
                root_path TEXT NOT NULL,
                status TEXT NOT NULL,
                setup_status TEXT NOT NULL,
                sandbox_backend TEXT NOT NULL,
                sandbox_status TEXT NOT NULL,
                fallback_reason TEXT,
                setup_script TEXT,
                audit_json TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                archived_at INTEGER,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL,
                FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_workspace_lifecycle_run
                ON workspace_lifecycle_runs(run_id);
            CREATE INDEX IF NOT EXISTS idx_workspace_lifecycle_session_updated
                ON workspace_lifecycle_runs(session_id, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_workspace_lifecycle_root
                ON workspace_lifecycle_runs(root_path);

            CREATE TABLE IF NOT EXISTS workspace_setup_events (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                stage TEXT NOT NULL,
                status TEXT NOT NULL,
                command TEXT,
                exit_code INTEGER,
                output_tail TEXT NOT NULL DEFAULT '',
                reason TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(workspace_id) REFERENCES workspace_lifecycle_runs(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_workspace_setup_events_workspace_created
                ON workspace_setup_events(workspace_id, created_at ASC);
            "#,
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (10, strftime('%s','now') * 1000)",
            [],
        )?;

        // v11 (M-11/M-12): eval harness run reports are durable records.
        // Case artifacts and command results are stored separately so a suite
        // pass can be audited without trusting a single summary blob.
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS eval_runs (
                id TEXT PRIMARY KEY,
                suite_id TEXT NOT NULL,
                provider TEXT,
                model TEXT,
                status TEXT NOT NULL,
                cwd TEXT NOT NULL,
                passed INTEGER NOT NULL DEFAULT 0,
                started_at INTEGER NOT NULL,
                finished_at INTEGER NOT NULL,
                report_json TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_eval_runs_suite_created
                ON eval_runs(suite_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_eval_runs_status_created
                ON eval_runs(status, created_at DESC);

            CREATE TABLE IF NOT EXISTS eval_case_results (
                id TEXT PRIMARY KEY,
                eval_run_id TEXT NOT NULL,
                case_id TEXT NOT NULL,
                status TEXT NOT NULL,
                passed INTEGER NOT NULL DEFAULT 0,
                verified INTEGER NOT NULL DEFAULT 0,
                false_completion INTEGER NOT NULL DEFAULT 0,
                blocked INTEGER NOT NULL DEFAULT 0,
                artifact_path TEXT,
                result_json TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL,
                FOREIGN KEY(eval_run_id) REFERENCES eval_runs(id) ON DELETE CASCADE,
                UNIQUE(eval_run_id, case_id)
            );

            CREATE INDEX IF NOT EXISTS idx_eval_case_results_run
                ON eval_case_results(eval_run_id, case_id);

            CREATE TABLE IF NOT EXISTS eval_command_results (
                id TEXT PRIMARY KEY,
                eval_run_id TEXT NOT NULL,
                case_id TEXT NOT NULL,
                command TEXT NOT NULL,
                cwd TEXT NOT NULL,
                required INTEGER NOT NULL DEFAULT 1,
                status TEXT NOT NULL,
                exit_code INTEGER,
                stdout_tail TEXT NOT NULL DEFAULT '',
                stderr_tail TEXT NOT NULL DEFAULT '',
                started_at INTEGER NOT NULL,
                finished_at INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                timed_out INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(eval_run_id) REFERENCES eval_runs(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_eval_command_results_run_case
                ON eval_command_results(eval_run_id, case_id);
            "#,
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (11, strftime('%s','now') * 1000)",
            [],
        )?;

        drop(conn);
        Ok(())
    }

    pub fn ensure_session(&self) -> StorageResult<SessionRecord> {
        if let Some(session) = self.list_sessions()?.into_iter().next() {
            Ok(session)
        } else {
            self.create_session("Initial session")
        }
    }

    pub fn list_sessions(&self) -> StorageResult<Vec<SessionRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, title, project_id, title_is_manual, pinned, archived_at, created_at, updated_at, COALESCE(last_active_at, updated_at)
             FROM sessions
             WHERE archived_at IS NULL
             ORDER BY pinned DESC, COALESCE(last_active_at, updated_at) DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                project_id: row.get(2)?,
                title_is_manual: row.get::<_, i64>(3)? != 0,
                pinned: row.get::<_, i64>(4)? != 0,
                archived_at: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                last_active_at: row.get(8)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn list_all_sessions(&self) -> StorageResult<Vec<SessionRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, title, project_id, title_is_manual, pinned, archived_at, created_at, updated_at, COALESCE(last_active_at, updated_at)
             FROM sessions
             ORDER BY archived_at IS NULL DESC, pinned DESC, COALESCE(archived_at, last_active_at, updated_at) DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                project_id: row.get(2)?,
                title_is_manual: row.get::<_, i64>(3)? != 0,
                pinned: row.get::<_, i64>(4)? != 0,
                archived_at: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                last_active_at: row.get(8)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn list_archived_sessions(&self) -> StorageResult<Vec<SessionRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, title, project_id, title_is_manual, pinned, archived_at, created_at, updated_at, COALESCE(last_active_at, updated_at)
             FROM sessions
             WHERE archived_at IS NOT NULL
             ORDER BY archived_at DESC, COALESCE(last_active_at, updated_at) DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                project_id: row.get(2)?,
                title_is_manual: row.get::<_, i64>(3)? != 0,
                pinned: row.get::<_, i64>(4)? != 0,
                archived_at: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                last_active_at: row.get(8)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn create_session(&self, title: &str) -> StorageResult<SessionRecord> {
        self.create_session_for_project(title, None)
    }

    pub fn create_session_for_project(
        &self,
        title: &str,
        project_id: Option<&str>,
    ) -> StorageResult<SessionRecord> {
        let project_id = project_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        if let Some(project_id) = project_id.as_deref() {
            self.get_project(project_id)?;
        }
        let now = now_ms();
        let session = SessionRecord {
            id: format!("s_{}", Uuid::new_v4()),
            title: if title.trim().is_empty() {
                "新会话".to_string()
            } else {
                title.trim().to_string()
            },
            project_id,
            title_is_manual: false,
            pinned: false,
            archived_at: None,
            created_at: now,
            updated_at: now,
            last_active_at: now,
        };

        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO sessions(id, title, project_id, title_is_manual, pinned, archived_at, created_at, updated_at, last_active_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                session.id,
                session.title,
                session.project_id,
                if session.title_is_manual { 1_i64 } else { 0_i64 },
                if session.pinned { 1_i64 } else { 0_i64 },
                session.archived_at,
                session.created_at,
                session.updated_at,
                session.last_active_at
            ],
        )?;
        Ok(session)
    }

    pub fn get_session(&self, id: &str) -> StorageResult<SessionRecord> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.query_row(
            "SELECT id, title, project_id, title_is_manual, pinned, archived_at, created_at, updated_at, COALESCE(last_active_at, updated_at)
             FROM sessions WHERE id = ?1",
            params![id],
            |row| {
                Ok(SessionRecord {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    project_id: row.get(2)?,
                    title_is_manual: row.get::<_, i64>(3)? != 0,
                    pinned: row.get::<_, i64>(4)? != 0,
                    archived_at: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    last_active_at: row.get(8)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    pub fn rename_session(&self, id: &str, title: &str) -> StorageResult<SessionRecord> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err(StorageError::Validation("会话名称不能为空。".to_string()));
        }
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "UPDATE sessions SET title = ?1, title_is_manual = 1, updated_at = ?2 WHERE id = ?3",
            params![trimmed, now, id],
        )?;
        drop(conn);
        self.get_session(id)
    }

    pub fn maybe_auto_title_session(
        &self,
        id: &str,
        candidate: &str,
    ) -> StorageResult<Option<SessionRecord>> {
        let current = self.get_session(id)?;
        if current.title_is_manual {
            return Ok(None);
        }
        let title = title_from_message(candidate);
        if title == "新会话" {
            return Ok(None);
        }
        let can_replace = matches!(
            current.title.trim(),
            "" | "新会话" | "New session" | "Initial session" | "旧会话"
        );
        if !can_replace {
            return Ok(None);
        }
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now, id],
        )?;
        drop(conn);
        self.get_session(id).map(Some)
    }

    pub fn search_sessions(&self, query: &str) -> StorageResult<Vec<SessionRecord>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return self.list_sessions();
        }
        let pattern = format!("%{}%", trimmed);
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT DISTINCT s.id, s.title, s.project_id, s.title_is_manual, s.pinned, s.archived_at, s.created_at, s.updated_at, COALESCE(s.last_active_at, s.updated_at)
             FROM sessions s
             LEFT JOIN messages m ON m.session_id = s.id
             WHERE s.archived_at IS NULL AND (s.title LIKE ?1 OR m.content LIKE ?1)
             ORDER BY s.pinned DESC, COALESCE(s.last_active_at, s.updated_at) DESC",
        )?;
        let rows = stmt.query_map(params![pattern], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                project_id: row.get(2)?,
                title_is_manual: row.get::<_, i64>(3)? != 0,
                pinned: row.get::<_, i64>(4)? != 0,
                archived_at: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                last_active_at: row.get(8)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn search_archived_sessions(&self, query: &str) -> StorageResult<Vec<SessionRecord>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return self.list_archived_sessions();
        }
        let pattern = format!("%{}%", trimmed);
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT DISTINCT s.id, s.title, s.project_id, s.title_is_manual, s.pinned, s.archived_at, s.created_at, s.updated_at, COALESCE(s.last_active_at, s.updated_at)
             FROM sessions s
             LEFT JOIN messages m ON m.session_id = s.id
             WHERE s.archived_at IS NOT NULL AND (s.title LIKE ?1 OR m.content LIKE ?1)
             ORDER BY s.archived_at DESC, COALESCE(s.last_active_at, s.updated_at) DESC",
        )?;
        let rows = stmt.query_map(params![pattern], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                project_id: row.get(2)?,
                title_is_manual: row.get::<_, i64>(3)? != 0,
                pinned: row.get::<_, i64>(4)? != 0,
                archived_at: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                last_active_at: row.get(8)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn set_session_pinned(&self, id: &str, pinned: bool) -> StorageResult<SessionRecord> {
        self.ensure_session_exists(id)?;
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "UPDATE sessions SET pinned = ?1, updated_at = ?2 WHERE id = ?3",
            params![if pinned { 1_i64 } else { 0_i64 }, now, id],
        )?;
        drop(conn);
        self.get_session(id)
    }

    pub fn archive_session(&self, id: &str) -> StorageResult<Vec<SessionRecord>> {
        self.ensure_session_exists(id)?;
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "UPDATE sessions SET archived_at = ?1, pinned = 0, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        drop(conn);
        if self.list_sessions()?.is_empty() {
            self.create_session("新会话")?;
        }
        self.list_sessions()
    }

    pub fn restore_session(&self, id: &str) -> StorageResult<SessionRecord> {
        self.ensure_session_exists(id)?;
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "UPDATE sessions SET archived_at = NULL, updated_at = ?1, last_active_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        drop(conn);
        self.get_session(id)
    }

    pub fn delete_session(&self, id: &str) -> StorageResult<Vec<SessionRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        drop(conn);

        if self.list_sessions()?.is_empty() {
            self.create_session("新会话")?;
        }
        self.list_sessions()
    }

    pub fn get_messages(&self, session_id: &str) -> StorageResult<Vec<MessageRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, created_at, metadata
             FROM messages WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            let metadata: String = row.get(5)?;
            Ok(MessageRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                metadata: serde_json::from_str(&metadata).unwrap_or_else(|_| json!({})),
            })
        })?;
        collect_rows(rows)
    }

    pub fn list_projects(&self) -> StorageResult<Vec<ProjectRecord>> {
        Ok(self
            .list_all_projects()?
            .into_iter()
            .filter(|project| !is_legacy_virtual_project(project) && project.archived_at.is_none())
            .collect())
    }

    pub fn list_all_projects(&self) -> StorageResult<Vec<ProjectRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, title, root_path, kind, pinned, archived_at, created_at, updated_at, last_active_at
             FROM projects ORDER BY pinned DESC, last_active_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ProjectRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                root_path: row.get(2)?,
                kind: row.get(3)?,
                pinned: row.get::<_, i64>(4)? != 0,
                archived_at: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                last_active_at: row.get(8)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn get_project(&self, id: &str) -> StorageResult<ProjectRecord> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.query_row(
            "SELECT id, title, root_path, kind, pinned, archived_at, created_at, updated_at, last_active_at
             FROM projects WHERE id = ?1",
            params![id],
            |row| {
                Ok(ProjectRecord {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    root_path: row.get(2)?,
                    kind: row.get(3)?,
                    pinned: row.get::<_, i64>(4)? != 0,
                    archived_at: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    last_active_at: row.get(8)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    pub fn upsert_project(
        &self,
        title: &str,
        root_path: Option<&str>,
        kind: &str,
    ) -> StorageResult<ProjectRecord> {
        let now = now_ms();
        let root_path = root_path
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let title = if title.trim().is_empty() {
            root_path
                .as_deref()
                .and_then(|path| Path::new(path).file_name())
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "未命名项目".to_string())
        } else {
            title.trim().to_string()
        };
        let kind = if kind.trim().is_empty() {
            "folder"
        } else {
            kind.trim()
        };

        if let Some(root) = root_path.as_deref() {
            let existing_id = {
                let conn = self.conn.lock().expect("local db mutex poisoned");
                conn.query_row(
                    "SELECT id FROM projects WHERE root_path = ?1",
                    params![root],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            };
            if let Some(id) = existing_id {
                let conn = self.conn.lock().expect("local db mutex poisoned");
                conn.execute(
                    "UPDATE projects SET title = ?1, kind = ?2, archived_at = NULL, updated_at = ?3, last_active_at = ?3 WHERE id = ?4",
                    params![title, kind, now, id],
                )?;
                drop(conn);
                return self.get_project(&id);
            }
        }

        let project = ProjectRecord {
            id: format!("p_{}", Uuid::new_v4()),
            title,
            root_path,
            kind: kind.to_string(),
            pinned: false,
            archived_at: None,
            created_at: now,
            updated_at: now,
            last_active_at: now,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO projects(id, title, root_path, kind, pinned, archived_at, created_at, updated_at, last_active_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                project.id,
                project.title,
                project.root_path,
                project.kind,
                project.pinned as i64,
                project.archived_at,
                project.created_at,
                project.updated_at,
                project.last_active_at
            ],
        )?;
        Ok(project)
    }

    pub fn rename_project(&self, id: &str, title: &str) -> StorageResult<ProjectRecord> {
        let title = title.trim();
        if title.is_empty() {
            return Err(StorageError::Validation("请输入项目名称。".to_string()));
        }
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let updated = conn.execute(
            "UPDATE projects SET title = ?1, updated_at = ?2, last_active_at = ?2 WHERE id = ?3",
            params![title, now, id],
        )?;
        drop(conn);
        if updated == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        self.get_project(id)
    }

    pub fn set_project_pinned(&self, id: &str, pinned: bool) -> StorageResult<ProjectRecord> {
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let updated = conn.execute(
            "UPDATE projects SET pinned = ?1, updated_at = ?2, last_active_at = ?2 WHERE id = ?3",
            params![pinned as i64, now, id],
        )?;
        drop(conn);
        if updated == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        self.get_project(id)
    }

    pub fn archive_project(&self, id: &str) -> StorageResult<ProjectRecord> {
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let updated = conn.execute(
            "UPDATE projects SET archived_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        drop(conn);
        if updated == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        self.get_project(id)
    }

    pub fn delete_project(&self, id: &str) -> StorageResult<Vec<ProjectRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let updated = conn.execute(
            "UPDATE sessions SET project_id = NULL, updated_at = ?1 WHERE project_id = ?2",
            params![now_ms(), id],
        )?;
        let deleted = conn.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        drop(conn);
        if deleted == 0 && updated == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        self.list_projects()
    }

    pub fn session_project_root(&self, session_id: &str) -> StorageResult<Option<PathBuf>> {
        let session = self.get_session(session_id)?;
        let Some(project_id) = session.project_id else {
            return Ok(None);
        };
        let project = self.get_project(&project_id)?;
        let Some(root) = project.root_path else {
            return Ok(None);
        };
        let path = PathBuf::from(root);
        if path.exists() {
            Ok(Some(path.canonicalize()?))
        } else {
            Ok(None)
        }
    }

    pub fn clear_session_context(&self, session_id: &str) -> StorageResult<i64> {
        self.ensure_session_exists(session_id)?;
        let now = now_ms();
        self.set_app_state(
            &format!("session_context_clear_after:{session_id}"),
            json!(now),
        )?;
        self.set_app_state(&format!("session_summary:{session_id}"), json!(null))?;
        Ok(now)
    }

    pub fn session_context_clear_after(&self, session_id: &str) -> StorageResult<Option<i64>> {
        Ok(self
            .get_app_state(&format!("session_context_clear_after:{session_id}"))?
            .and_then(|value| value.as_i64()))
    }

    pub fn save_message(
        &self,
        session_id: &str,
        message: SaveMessagePayload,
    ) -> StorageResult<MessageRecord> {
        self.ensure_session_exists(session_id)?;
        let now = now_ms();
        let record = MessageRecord {
            id: message
                .id
                .unwrap_or_else(|| format!("msg_{}", Uuid::new_v4())),
            session_id: session_id.to_string(),
            role: message.role,
            content: message.content,
            created_at: message.created_at.unwrap_or(now),
            metadata: if message.metadata.is_null() {
                json!({})
            } else {
                message.metadata
            },
        };

        let metadata = serde_json::to_string(&record.metadata)?;
        let role_for_title = record.role.clone();
        let content_for_title = record.content.clone();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT OR REPLACE INTO messages(id, session_id, role, content, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.id,
                record.session_id,
                record.role,
                record.content,
                metadata,
                record.created_at
            ],
        )?;
        conn.execute(
            "UPDATE sessions SET updated_at = ?1, last_active_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        drop(conn);
        if role_for_title == "user" {
            let _ = self.maybe_auto_title_session(session_id, &content_for_title);
        }
        Ok(record)
    }

    pub fn list_memories(&self) -> StorageResult<Vec<MemoryRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, text, source, enabled, quality, confidence, last_used_at, use_count, created_at, updated_at
             FROM memories ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(MemoryRecord {
                id: row.get(0)?,
                text: row.get(1)?,
                source: row.get(2)?,
                enabled: row.get::<_, i64>(3)? != 0,
                quality: row.get(4)?,
                confidence: row.get(5)?,
                last_used_at: row.get(6)?,
                use_count: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn add_memory(&self, text: &str, source: &str) -> StorageResult<MemoryRecord> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(StorageError::NotFound("memory text is empty".to_string()));
        }
        let normalized = normalize_memory_text(trimmed);
        if let Some(existing) = self
            .list_memories()?
            .into_iter()
            .find(|memory| normalize_memory_text(&memory.text) == normalized)
        {
            let now = now_ms();
            let conn = self.conn.lock().expect("local db mutex poisoned");
            conn.execute(
                "UPDATE memories
                 SET enabled = 1, quality = 'confirmed', confidence = MAX(confidence, 1.0), updated_at = ?1
                 WHERE id = ?2",
                params![now, existing.id],
            )?;
            drop(conn);
            return self
                .list_memories()?
                .into_iter()
                .find(|memory| normalize_memory_text(&memory.text) == normalized)
                .ok_or_else(|| StorageError::NotFound("memory dedupe record".to_string()));
        }

        let now = now_ms();
        let memory = MemoryRecord {
            id: format!("m_{}", Uuid::new_v4()),
            text: trimmed.to_string(),
            source: if source.trim().is_empty() {
                "manual".to_string()
            } else {
                source.trim().to_string()
            },
            enabled: true,
            quality: "confirmed".to_string(),
            confidence: 1.0,
            last_used_at: None,
            use_count: 0,
            created_at: now,
            updated_at: now,
        };

        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO memories(id, text, source, enabled, quality, confidence, last_used_at, use_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                memory.id,
                memory.text,
                memory.source,
                1_i64,
                memory.quality,
                memory.confidence,
                memory.last_used_at,
                memory.use_count,
                memory.created_at,
                memory.updated_at
            ],
        )?;
        Ok(memory)
    }

    pub fn update_memory(
        &self,
        id: &str,
        text: Option<String>,
        enabled: Option<bool>,
    ) -> StorageResult<MemoryRecord> {
        let current = self
            .list_memories()?
            .into_iter()
            .find(|memory| memory.id == id)
            .ok_or_else(|| StorageError::NotFound(id.to_string()))?;
        let has_text_update = text.is_some();
        let next_text = text.unwrap_or_else(|| current.text.clone());
        let next_enabled = enabled.unwrap_or(current.enabled);
        let reconfirm = next_enabled
            && (!current.enabled
                || current.quality == "decayed"
                || current.confidence < 1.0
                || has_text_update);
        let now = now_ms();

        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "UPDATE memories
             SET text = ?1,
                 enabled = ?2,
                 quality = CASE WHEN ?5 = 1 THEN 'confirmed' ELSE quality END,
                 confidence = CASE WHEN ?5 = 1 THEN 1.0 ELSE confidence END,
                 updated_at = ?3
             WHERE id = ?4",
            params![
                next_text,
                if next_enabled { 1_i64 } else { 0_i64 },
                now,
                id,
                if reconfirm { 1_i64 } else { 0_i64 },
            ],
        )?;
        drop(conn);

        self.list_memories()?
            .into_iter()
            .find(|memory| memory.id == id)
            .ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    /// Records that all enabled memories were injected into context this turn.
    /// P2-5: bumps `use_count`/`last_used_at` only — deliberately NOT `updated_at`,
    /// which is the decay clock (see [`Self::decay_memories`]). Injection is "use",
    /// not "reinforcement", so it must not keep a stale memory alive forever.
    pub fn mark_enabled_memories_used(&self) -> StorageResult<usize> {
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE memories
             SET use_count = COALESCE(use_count, 0) + 1,
                 last_used_at = ?1
             WHERE enabled = 1",
            params![now],
        )?;
        Ok(changed)
    }

    /// P2-5: decays confidence for memories that have not been re-confirmed
    /// (created/edited/re-added) within `idle_ms`. Reference timestamp is
    /// `updated_at`; recently reinforced memories are left untouched so stable,
    /// repeatedly-confirmed rules survive. Confidence is clamped at 0. Returns the
    /// number of rows whose confidence was lowered.
    pub fn decay_memories(&self, now: i64, idle_ms: i64, step: f64) -> StorageResult<usize> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE memories
             SET confidence = MAX(0.0, confidence - ?1)
             WHERE confidence > 0.0
               AND (?2 - COALESCE(updated_at, created_at)) > ?3",
            params![step, now, idle_ms],
        )?;
        Ok(changed)
    }

    /// P2-5: soft-deletes memories whose confidence decayed below `floor`. Rows are
    /// kept (only `enabled` flips to 0, `quality` marked `decayed`) so a wrongly
    /// purged memory stays recoverable via [`Self::update_memory`]. Returns the
    /// number of memories soft-disabled.
    pub fn purge_low_confidence_memories(&self, floor: f64) -> StorageResult<usize> {
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE memories
             SET enabled = 0, quality = 'decayed', updated_at = ?1
             WHERE enabled = 1 AND confidence < ?2",
            params![now, floor],
        )?;
        Ok(changed)
    }

    /// P2-5: one decay + purge maintenance pass with the default policy. Called on
    /// the memory-injection path so decay/purge are live (not dead code) while the
    /// granular [`Self::decay_memories`]/[`Self::purge_low_confidence_memories`]
    /// stay testable with explicit clocks.
    pub fn maintain_memories(&self) -> StorageResult<usize> {
        self.decay_memories(now_ms(), MEMORY_DECAY_IDLE_MS, MEMORY_DECAY_STEP)?;
        self.purge_low_confidence_memories(MEMORY_PURGE_FLOOR)
    }

    pub fn delete_memory(&self, id: &str) -> StorageResult<()> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn clear_memories(&self) -> StorageResult<()> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute("DELETE FROM memories", [])?;
        Ok(())
    }

    pub fn get_profile(&self) -> StorageResult<ProfileRecord> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let record = conn
            .query_row(
                "SELECT id, profile_json, updated_at FROM profiles WHERE id = 'default'",
                [],
                |row| {
                    let profile_json: String = row.get(1)?;
                    Ok(ProfileRecord {
                        id: row.get(0)?,
                        profile: serde_json::from_str(&profile_json)
                            .unwrap_or_else(|_| default_profile_json()),
                        updated_at: row.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(record.unwrap_or(ProfileRecord {
            id: "default".to_string(),
            profile: default_profile_json(),
            updated_at: now_ms(),
        }))
    }

    pub fn save_profile(&self, profile: serde_json::Value) -> StorageResult<ProfileRecord> {
        let now = now_ms();
        let profile_string = serde_json::to_string_pretty(&profile)?;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT OR REPLACE INTO profiles(id, profile_json, updated_at) VALUES ('default', ?1, ?2)",
            params![profile_string, now],
        )?;
        drop(conn);

        export_profile_markdown(&profile)?;
        self.get_profile()
    }

    pub fn get_personality_progress(&self) -> StorageResult<PersonalityProgressRecord> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let record = conn
            .query_row(
                "SELECT id, progress_json, updated_at FROM personality_progress WHERE id = 'default'",
                [],
                |row| {
                    let progress_json: String = row.get(1)?;
                    Ok(PersonalityProgressRecord {
                        id: row.get(0)?,
                        progress: serde_json::from_str(&progress_json).unwrap_or_else(|_| json!({})),
                        updated_at: row.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(record.unwrap_or(PersonalityProgressRecord {
            id: "default".to_string(),
            progress: json!({ "status": "not_started", "answers": [] }),
            updated_at: now_ms(),
        }))
    }

    pub fn save_personality_progress(
        &self,
        progress: serde_json::Value,
    ) -> StorageResult<PersonalityProgressRecord> {
        let now = now_ms();
        let progress_string = serde_json::to_string(&progress)?;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT OR REPLACE INTO personality_progress(id, progress_json, updated_at) VALUES ('default', ?1, ?2)",
            params![progress_string, now],
        )?;
        drop(conn);
        self.get_personality_progress()
    }

    pub fn complete_personality_test(
        &self,
        answers: serde_json::Value,
    ) -> StorageResult<ProfileRecord> {
        let scores = score_personality(&answers);
        let verbosity = scores
            .get("verbosity")
            .and_then(|v| v.as_i64())
            .unwrap_or(3);
        let support = scores
            .get("supportMode")
            .and_then(|v| v.as_i64())
            .unwrap_or(3);
        let proactivity = scores
            .get("proactivity")
            .and_then(|v| v.as_i64())
            .unwrap_or(3);
        let ati_profile = build_ati_profile(&scores);
        let profile = json!({
            "personalityType": ati_profile.get("code").and_then(|value| value.as_str()).unwrap_or("ATLAS-LOCAL"),
            "atiProfile": ati_profile,
            "dimensionScores": scores,
            "replyStyle": if verbosity <= 2 { "minimal" } else if support >= 4 { "gentle" } else { "professional" },
            "tonePreference": if support >= 4 { "supportive" } else { "natural" },
            "verbosity": verbosity,
            "supportMode": if support >= 4 { "emotion_first" } else { "solution_first" },
            "proactivity": if proactivity >= 4 { "high" } else { "balanced" },
            "interests": infer_interests(&answers),
            "testCompletedAt": Utc::now().to_rfc3339(),
            "testVersion": 1,
            "rawAnswers": answers,
        });
        self.save_personality_progress(json!({ "status": "completed", "profile": profile }))?;
        self.save_profile(profile)
    }

    pub fn prepare_file_write(
        &self,
        target_path: PathBuf,
        content: String,
        reason: String,
    ) -> StorageResult<FileWritePreview> {
        let path = normalize_write_target(&target_path)?;
        validate_write_target(&path)?;
        if content.len() > 1_000_000 {
            return Err(StorageError::Validation(
                "文件内容超过 1MB，Atlas 不会一次性写入这么大的文本。".to_string(),
            ));
        }
        let existing_text = if path.exists() {
            std::fs::read_to_string(&path).ok()
        } else {
            None
        };
        let existing_preview = existing_text
            .as_deref()
            .map(|text| compact_text(text, 1600));
        let id = format!("pfw_{}", Uuid::new_v4());
        let now = now_ms();
        let preview = FileWritePreview {
            id: id.clone(),
            target_path: path.to_string_lossy().to_string(),
            operation: if path.exists() { "overwrite" } else { "create" }.to_string(),
            content_size: content.len(),
            preview: compact_text(&content, 4000),
            existing_preview,
            diff: Some(file_write_diff(existing_text.as_deref(), &content)),
            reason: compact_text(&reason, 300),
            created_at: now,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO pending_file_writes(id, target_path, content, reason, status, created_at, resolved_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5, NULL)",
            params![id, &preview.target_path, content, &preview.reason, now],
        )?;
        drop(conn);
        self.log_activity_event(LogActivityEventPayload {
            date: None,
            kind: "system".to_string(),
            title: "准备写入文件".to_string(),
            detail: format!("等待确认：{}", preview.target_path),
            metadata: json!({ "pendingWriteId": preview.id, "operation": preview.operation }),
        })?;
        Ok(preview)
    }

    pub fn confirm_pending_file_write(
        &self,
        id: &str,
        override_text: Option<&str>,
    ) -> StorageResult<FileWritePreview> {
        let (target, content, reason, created_at) = {
            let conn = self.conn.lock().expect("local db mutex poisoned");
            conn.query_row(
                "SELECT target_path, content, reason, created_at FROM pending_file_writes WHERE id = ?1 AND status = 'pending'",
                params![id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)),
            )
            .optional()?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))?
        };
        let path = normalize_write_target(&PathBuf::from(&target))?;
        validate_write_target(&path)?;
        let final_content = override_text.unwrap_or(&content);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let existing_text = if path.exists() {
            std::fs::read_to_string(&path).ok()
        } else {
            None
        };
        let existing_preview = existing_text
            .as_deref()
            .map(|text| compact_text(text, 1600));
        std::fs::write(&path, final_content)?;
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "UPDATE pending_file_writes SET status = 'approved', resolved_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        drop(conn);
        self.log_activity_event(LogActivityEventPayload {
            date: None,
            kind: "system".to_string(),
            title: "已写入文件".to_string(),
            detail: path.to_string_lossy().to_string(),
            metadata: json!({ "pendingWriteId": id }),
        })?;
        let preview = FileWritePreview {
            id: id.to_string(),
            target_path: path.to_string_lossy().to_string(),
            operation: if existing_preview.is_some() {
                "overwrite"
            } else {
                "create"
            }
            .to_string(),
            content_size: final_content.len(),
            preview: compact_text(final_content, 4000),
            existing_preview,
            diff: Some(file_write_diff(existing_text.as_deref(), final_content)),
            reason,
            created_at,
        };
        let _ = self.record_artifact(RecordArtifactPayload {
            session_id: None,
            run_id: None,
            kind: "file".to_string(),
            title: Path::new(&preview.target_path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("文件")
                .to_string(),
            path: Some(preview.target_path.clone()),
            operation: preview.operation.clone(),
            status: "written".to_string(),
            summary: format!("已写入文件：{}", preview.target_path),
            metadata: json!({
                "pendingWriteId": id,
                "contentSize": preview.content_size,
                "hasDiff": preview.diff.is_some()
            }),
        });
        Ok(preview)
    }

    pub fn reject_pending_file_write(&self, id: &str) -> StorageResult<()> {
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE pending_file_writes SET status = 'rejected', resolved_at = ?1 WHERE id = ?2 AND status = 'pending'",
            params![now, id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        Ok(())
    }

    pub fn prepare_command(
        &self,
        command: String,
        cwd: String,
        reason: String,
        shell: String,
    ) -> StorageResult<PendingCommandRecord> {
        let id = format!("pcmd_{}", Uuid::new_v4());
        let now = now_ms();
        let record = PendingCommandRecord {
            id,
            command: compact_text(&command, 4_000),
            cwd: compact_text(&cwd, 1_000),
            reason: compact_text(&reason, 300),
            shell: compact_text(&shell, 80),
            created_at: now,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO pending_commands(id, command, cwd, reason, shell, status, created_at, resolved_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, NULL)",
            params![
                &record.id,
                &record.command,
                &record.cwd,
                &record.reason,
                &record.shell,
                record.created_at
            ],
        )?;
        Ok(record)
    }

    pub fn confirm_pending_command(&self, id: &str) -> StorageResult<PendingCommandRecord> {
        let record = {
            let conn = self.conn.lock().expect("local db mutex poisoned");
            conn.query_row(
                "SELECT id, command, cwd, reason, shell, created_at FROM pending_commands WHERE id = ?1 AND status = 'pending'",
                params![id],
                pending_command_from_row,
            )
            .optional()?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))?
        };
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "UPDATE pending_commands SET status = 'approved', resolved_at = ?1 WHERE id = ?2 AND status = 'pending'",
            params![now, id],
        )?;
        Ok(record)
    }

    pub fn reject_pending_command(&self, id: &str) -> StorageResult<PendingCommandRecord> {
        let record = {
            let conn = self.conn.lock().expect("local db mutex poisoned");
            conn.query_row(
                "SELECT id, command, cwd, reason, shell, created_at FROM pending_commands WHERE id = ?1 AND status = 'pending'",
                params![id],
                pending_command_from_row,
            )
            .optional()?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))?
        };
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE pending_commands SET status = 'rejected', resolved_at = ?1 WHERE id = ?2 AND status = 'pending'",
            params![now, id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        Ok(record)
    }

    pub fn find_run_id_for_pending_command(
        &self,
        pending_command_id: &str,
    ) -> StorageResult<Option<String>> {
        let needle = format!("%{pending_command_id}%");
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT run_id, output_json
             FROM agent_run_steps
             WHERE step_type = 'approval' AND output_json LIKE ?1
             ORDER BY created_at DESC
             LIMIT 20",
        )?;
        let rows = stmt.query_map(params![needle], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (run_id, output_json) = row?;
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&output_json) else {
                continue;
            };
            let id = value
                .get("data")
                .and_then(|data| data.get("pendingCommand"))
                .and_then(|pending| pending.get("id"))
                .and_then(|value| value.as_str());
            if id == Some(pending_command_id) {
                return Ok(Some(run_id));
            }
        }
        Ok(None)
    }

    pub fn get_app_state(&self, key: &str) -> StorageResult<Option<serde_json::Value>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let value = conn
            .query_row(
                "SELECT value_json FROM app_state WHERE key = ?1",
                params![key],
                |row| {
                    let raw: String = row.get(0)?;
                    Ok(serde_json::from_str(&raw).unwrap_or(json!(null)))
                },
            )
            .optional()?;
        Ok(value)
    }

    pub fn set_app_state(&self, key: &str, value: serde_json::Value) -> StorageResult<()> {
        let now = now_ms();
        let raw = serde_json::to_string(&value)?;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT OR REPLACE INTO app_state(key, value_json, updated_at) VALUES (?1, ?2, ?3)",
            params![key, raw, now],
        )?;
        Ok(())
    }

    pub fn get_session_summary(&self, session_id: &str) -> StorageResult<Option<SessionSummary>> {
        let key = session_summary_key(session_id);
        Ok(self
            .get_app_state(&key)?
            .and_then(|value| serde_json::from_value(value).ok()))
    }

    pub fn save_session_summary(&self, summary: SessionSummary) -> StorageResult<SessionSummary> {
        self.set_app_state(
            &session_summary_key(&summary.session_id),
            serde_json::to_value(&summary)?,
        )?;
        Ok(summary)
    }

    pub fn summarize_session(&self, session_id: &str) -> StorageResult<SessionSummary> {
        let messages = self.get_messages(session_id)?;
        let text_messages: Vec<&MessageRecord> = messages
            .iter()
            .filter(|message| matches!(message.role.as_str(), "user" | "assistant"))
            .collect();
        let source_message_count = text_messages.len();
        let take = text_messages.len().saturating_sub(12);
        let summary_source = if take == 0 {
            text_messages.as_slice()
        } else {
            &text_messages[..take]
        };
        let mut lines = Vec::new();
        for message in summary_source.iter().rev().take(24).rev() {
            let role = if message.role == "user" {
                "用户"
            } else {
                "Atlas"
            };
            lines.push(format!("{role}: {}", compact_text(&message.content, 140)));
        }
        let summary = if lines.is_empty() {
            "暂无可压缩的历史内容。".to_string()
        } else {
            format!("以下是本会话较早内容的压缩摘要：\n{}", lines.join("\n"))
        };
        self.save_session_summary(SessionSummary {
            session_id: session_id.to_string(),
            summary,
            source_message_count,
            updated_at: now_ms(),
        })
    }

    pub fn conversation_history_report(
        &self,
        range: Option<&str>,
    ) -> StorageResult<ConversationHistoryReport> {
        let now = now_ms();
        let (range_label, cutoff) = parse_history_report_range(range, now);
        let sessions = self.list_all_sessions()?;
        let projects = self.list_all_projects()?;
        let mut included: Vec<(SessionRecord, Vec<MessageRecord>)> = Vec::new();

        for session in sessions {
            let messages = self.get_messages(&session.id)?;
            let scoped_messages: Vec<MessageRecord> = messages
                .into_iter()
                .filter(|message| {
                    if !matches!(message.role.as_str(), "user" | "assistant" | "system") {
                        return false;
                    }
                    cutoff.is_none_or(|start| message.created_at >= start)
                })
                .collect();
            if scoped_messages.is_empty() {
                continue;
            }
            included.push((session, scoped_messages));
        }

        included.sort_by(|a, b| {
            let a_time =
                a.1.last()
                    .map(|message| message.created_at)
                    .unwrap_or(a.0.last_active_at);
            let b_time =
                b.1.last()
                    .map(|message| message.created_at)
                    .unwrap_or(b.0.last_active_at);
            b_time.cmp(&a_time)
        });

        let message_count = included.iter().map(|(_, messages)| messages.len()).sum();
        let user_message_count = included
            .iter()
            .flat_map(|(_, messages)| messages.iter())
            .filter(|message| message.role == "user")
            .count();
        let assistant_message_count = included
            .iter()
            .flat_map(|(_, messages)| messages.iter())
            .filter(|message| message.role == "assistant")
            .count();

        let mut lines = vec![
            format!("历史对话报告（{}）", range_label),
            String::new(),
            format!("- 会话：{}", included.len()),
            format!(
                "- 消息：{}（用户 {} / Atlas {}）",
                message_count, user_message_count, assistant_message_count
            ),
        ];

        let project_session_count = included
            .iter()
            .filter(|(session, _)| session.project_id.is_some())
            .count();
        if project_session_count > 0 {
            lines.push(format!("- 项目会话：{}", project_session_count));
        }

        if included.is_empty() {
            lines.push(String::new());
            lines.push("这段时间内没有可汇总的历史对话。".to_string());
        } else {
            lines.push(String::new());
            lines.push("主要对话：".to_string());
            for (index, (session, messages)) in included.iter().take(8).enumerate() {
                let project_label = session
                    .project_id
                    .as_deref()
                    .and_then(|id| projects.iter().find(|project| project.id == id))
                    .map(|project| format!(" · {}", project.title))
                    .unwrap_or_default();
                let first_user = messages
                    .iter()
                    .find(|message| message.role == "user")
                    .map(|message| compact_text(&message.content, 80))
                    .unwrap_or_else(|| compact_text(&session.title, 80));
                lines.push(format!(
                    "{}. {}{}：{} 条消息；{}",
                    index + 1,
                    compact_text(&session.title, 36),
                    project_label,
                    messages.len(),
                    first_user
                ));
            }

            let mut recent_user_records: Vec<&MessageRecord> = included
                .iter()
                .flat_map(|(_, messages)| messages.iter())
                .filter(|message| message.role == "user")
                .collect();
            recent_user_records.sort_by_key(|e| std::cmp::Reverse(e.created_at));
            let recent_user_messages: Vec<String> = recent_user_records
                .into_iter()
                .take(10)
                .map(|message| format!("- {}", compact_text(&message.content, 110)))
                .collect();
            if !recent_user_messages.is_empty() {
                lines.push(String::new());
                lines.push("最近用户意图：".to_string());
                lines.extend(recent_user_messages);
            }

            let mut open_question_records: Vec<&MessageRecord> = included
                .iter()
                .flat_map(|(_, messages)| messages.iter())
                .filter(|message| message.role == "user" && looks_like_question(&message.content))
                .collect();
            open_question_records.sort_by_key(|e| std::cmp::Reverse(e.created_at));
            let open_questions: Vec<String> = open_question_records
                .into_iter()
                .take(5)
                .map(|message| format!("- {}", compact_text(&message.content, 110)))
                .collect();
            if !open_questions.is_empty() {
                lines.push(String::new());
                lines.push("可回看的问题：".to_string());
                lines.extend(open_questions);
            }
        }

        Ok(ConversationHistoryReport {
            range_label,
            session_count: included.len(),
            message_count,
            user_message_count,
            assistant_message_count,
            report: lines.join("\n"),
            generated_at: now,
        })
    }

    pub fn log_activity_event(
        &self,
        payload: LogActivityEventPayload,
    ) -> StorageResult<ActivityEvent> {
        let title = payload.title.trim();
        if title.is_empty() {
            return Err(StorageError::NotFound(
                "activity title is empty".to_string(),
            ));
        }
        let now = now_ms();
        let event = ActivityEvent {
            id: format!("act_{}", Uuid::new_v4()),
            date: normalize_date(payload.date.as_deref().unwrap_or("")),
            kind: normalize_activity_kind(&payload.kind),
            title: compact_text(title, 120),
            detail: compact_text(payload.detail.trim(), 260),
            metadata: if payload.metadata.is_null() {
                json!({})
            } else {
                payload.metadata
            },
            created_at: now,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO activity_events(id, date, kind, title, detail, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &event.id,
                &event.date,
                &event.kind,
                &event.title,
                &event.detail,
                serde_json::to_string(&event.metadata)?,
                event.created_at
            ],
        )?;
        Ok(event)
    }

    pub fn recent_activity_events(
        &self,
        date: Option<&str>,
        limit: usize,
    ) -> StorageResult<Vec<ActivityEvent>> {
        let limit = limit.clamp(1, 100) as i64;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        if let Some(date) = date {
            let date = normalize_date(date);
            let mut stmt = conn.prepare(
                "SELECT id, date, kind, title, detail, metadata, created_at
                 FROM activity_events WHERE date = ?1 ORDER BY created_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![date, limit], activity_event_from_row)?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, date, kind, title, detail, metadata, created_at
                 FROM activity_events ORDER BY created_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], activity_event_from_row)?;
            collect_rows(rows)
        }
    }

    pub fn log_agent_tool_audit_event(
        &self,
        payload: LogAgentToolAuditPayload,
    ) -> StorageResult<AgentToolAuditRecord> {
        let tool_name = payload.tool_name.trim();
        if tool_name.is_empty() {
            return Err(StorageError::Validation(
                "audit tool name is empty".to_string(),
            ));
        }
        let run_id = payload.run_id.trim();
        if run_id.is_empty() {
            return Err(StorageError::Validation(
                "audit run id is empty".to_string(),
            ));
        }

        let record = AgentToolAuditRecord {
            id: format!("ata_{}", Uuid::new_v4()),
            session_id: payload.session_id.and_then(|value| {
                let trimmed = compact_text(value.trim(), 80);
                (!trimmed.is_empty()).then_some(trimmed)
            }),
            run_id: compact_text(run_id, 96),
            iteration: payload.iteration as i64,
            tool_call_id: compact_text(payload.tool_call_id.trim(), 96),
            tool_name: compact_text(tool_name, 96),
            permission_mode: normalize_agent_permission_mode(&payload.permission_mode),
            policy: normalize_tool_policy_name(&payload.policy),
            status: normalize_agent_tool_audit_status(&payload.status),
            reason: compact_text(payload.reason.trim(), 96),
            created_at: now_ms(),
        };

        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO agent_tool_audit_events(
                id, session_id, run_id, iteration, tool_call_id, tool_name,
                permission_mode, policy, status, reason, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                &record.id,
                record.session_id.as_deref(),
                &record.run_id,
                record.iteration,
                &record.tool_call_id,
                &record.tool_name,
                &record.permission_mode,
                &record.policy,
                &record.status,
                &record.reason,
                record.created_at,
            ],
        )?;
        conn.execute(
            "DELETE FROM agent_tool_audit_events
             WHERE id NOT IN (
                SELECT id FROM agent_tool_audit_events
                ORDER BY created_at DESC
                LIMIT ?1
             )",
            params![AGENT_TOOL_AUDIT_RETENTION_ROWS],
        )?;
        Ok(record)
    }

    pub fn recent_agent_tool_audit_events(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> StorageResult<Vec<AgentToolAuditRecord>> {
        let limit = limit.clamp(1, 200) as i64;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
            let session_id = compact_text(session_id.trim(), 80);
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, iteration, tool_call_id, tool_name,
                        permission_mode, policy, status, reason, created_at
                 FROM agent_tool_audit_events
                 WHERE session_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![session_id, limit], agent_tool_audit_from_row)?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, iteration, tool_call_id, tool_name,
                        permission_mode, policy, status, reason, created_at
                 FROM agent_tool_audit_events
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], agent_tool_audit_from_row)?;
            collect_rows(rows)
        }
    }

    /// P0-4: record one structured permission decision in its own table.
    pub fn log_permission_decision(
        &self,
        payload: LogPermissionDecisionPayload,
    ) -> StorageResult<PermissionDecisionRecord> {
        let run_id = payload.run_id.trim();
        if run_id.is_empty() {
            return Err(StorageError::Validation(
                "permission decision run id is empty".to_string(),
            ));
        }
        let action = payload.action.trim();
        if action.is_empty() {
            return Err(StorageError::Validation(
                "permission decision action is empty".to_string(),
            ));
        }

        let record = PermissionDecisionRecord {
            id: format!("pd_{}", Uuid::new_v4()),
            session_id: payload.session_id.and_then(|value| {
                let trimmed = compact_text(value.trim(), 80);
                (!trimmed.is_empty()).then_some(trimmed)
            }),
            run_id: compact_text(run_id, 96),
            iteration: payload.iteration as i64,
            tool_call_id: compact_text(payload.tool_call_id.trim(), 96),
            subject: normalize_permission_subject(&payload.subject),
            action: compact_text(action, 96),
            risk: normalize_permission_risk(&payload.risk),
            mode: normalize_agent_permission_mode(&payload.mode),
            decision: normalize_permission_decision(&payload.decision),
            reason: compact_text(payload.reason.trim(), 200),
            decided_by: normalize_permission_decided_by(&payload.decided_by),
            created_at: now_ms(),
        };

        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO permission_decisions(
                id, session_id, run_id, iteration, tool_call_id, subject,
                action, risk, mode, decision, reason, decided_by, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                &record.id,
                record.session_id.as_deref(),
                &record.run_id,
                record.iteration,
                &record.tool_call_id,
                &record.subject,
                &record.action,
                &record.risk,
                &record.mode,
                &record.decision,
                &record.reason,
                &record.decided_by,
                record.created_at,
            ],
        )?;
        conn.execute(
            "DELETE FROM permission_decisions
             WHERE id NOT IN (
                SELECT id FROM permission_decisions
                ORDER BY created_at DESC
                LIMIT ?1
             )",
            params![PERMISSION_DECISION_RETENTION_ROWS],
        )?;
        Ok(record)
    }

    /// P0-4: all permission decisions for a run, oldest first (the run's
    /// decision timeline — answers 谁批 / 为什么 / 何时).
    pub fn permission_decisions_for_run(
        &self,
        run_id: &str,
        limit: usize,
    ) -> StorageResult<Vec<PermissionDecisionRecord>> {
        let run_id = run_id.trim();
        if run_id.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 500) as i64;
        let run_id = compact_text(run_id, 96);
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, run_id, iteration, tool_call_id, subject,
                    action, risk, mode, decision, reason, decided_by, created_at
             FROM permission_decisions
             WHERE run_id = ?1
             ORDER BY created_at ASC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![run_id, limit], permission_decision_from_row)?;
        collect_rows(rows)
    }

    pub fn log_model_usage_event(
        &self,
        payload: LogModelUsagePayload,
    ) -> StorageResult<ModelUsageRecord> {
        let run_id = compact_text(payload.run_id.trim(), 96);
        if run_id.is_empty() {
            return Err(StorageError::Validation(
                "model usage run id is empty".to_string(),
            ));
        }
        let input_tokens = payload.input_tokens.max(0);
        let output_tokens = payload.output_tokens.max(0);
        let total_tokens = payload.total_tokens.max(input_tokens + output_tokens);
        if total_tokens <= 0 {
            return Err(StorageError::Validation(
                "model usage token total is empty".to_string(),
            ));
        }
        let record = ModelUsageRecord {
            id: format!("usage_{}", Uuid::new_v4()),
            session_id: payload.session_id.and_then(|value| {
                let trimmed = compact_text(value.trim(), 96);
                (!trimmed.is_empty()).then_some(trimmed)
            }),
            run_id,
            iteration: payload.iteration as i64,
            provider: compact_text(payload.provider.trim(), 80),
            model: compact_text(payload.model.trim(), 160),
            input_tokens,
            output_tokens,
            total_tokens,
            source: compact_text(payload.source.trim(), 80),
            created_at: now_ms(),
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO model_usage_events(
                id, session_id, run_id, iteration, provider, model,
                input_tokens, output_tokens, total_tokens, source, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                &record.id,
                record.session_id.as_deref(),
                &record.run_id,
                record.iteration,
                &record.provider,
                &record.model,
                record.input_tokens,
                record.output_tokens,
                record.total_tokens,
                &record.source,
                record.created_at,
            ],
        )?;
        Ok(record)
    }

    pub fn recent_model_usage_events(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> StorageResult<Vec<ModelUsageRecord>> {
        let limit = limit.clamp(1, 200) as i64;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
            let session_id = compact_text(session_id.trim(), 96);
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, iteration, provider, model,
                        input_tokens, output_tokens, total_tokens, source, created_at
                 FROM model_usage_events
                 WHERE session_id = ?1 AND source = 'model_api_usage'
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![session_id, limit], model_usage_from_row)?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, iteration, provider, model,
                        input_tokens, output_tokens, total_tokens, source, created_at
                 FROM model_usage_events
                 WHERE source = 'model_api_usage'
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], model_usage_from_row)?;
            collect_rows(rows)
        }
    }

    pub fn model_usage_summary(
        &self,
        session_id: Option<&str>,
    ) -> StorageResult<ModelUsageSummary> {
        let recent = self.recent_model_usage_events(session_id, 50)?;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let query = if session_id
            .filter(|value| !value.trim().is_empty())
            .is_some()
        {
            "SELECT COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), COALESCE(SUM(total_tokens), 0)
             FROM model_usage_events WHERE session_id = ?1 AND source = 'model_api_usage'"
        } else {
            "SELECT COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), COALESCE(SUM(total_tokens), 0)
             FROM model_usage_events WHERE source = 'model_api_usage'"
        };
        let summary = if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty())
        {
            let session_id = compact_text(session_id.trim(), 96);
            conn.query_row(query, params![session_id], |row| {
                Ok(ModelUsageSummary {
                    events: row.get(0)?,
                    input_tokens: row.get(1)?,
                    output_tokens: row.get(2)?,
                    total_tokens: row.get(3)?,
                    recent: recent.clone(),
                })
            })?
        } else {
            conn.query_row(query, [], |row| {
                Ok(ModelUsageSummary {
                    events: row.get(0)?,
                    input_tokens: row.get(1)?,
                    output_tokens: row.get(2)?,
                    total_tokens: row.get(3)?,
                    recent: recent.clone(),
                })
            })?
        };
        Ok(summary)
    }

    pub fn model_usage_total_for_run(&self, run_id: &str) -> StorageResult<i64> {
        let run_id = compact_text(run_id.trim(), 96);
        if run_id.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.query_row(
            "SELECT COALESCE(SUM(total_tokens), 0)
             FROM model_usage_events
             WHERE run_id = ?1 AND source = 'model_api_usage'",
            params![run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(Into::into)
    }

    pub fn model_usage_total_for_session(&self, session_id: &str) -> StorageResult<i64> {
        let session_id = compact_text(session_id.trim(), 96);
        if session_id.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.query_row(
            "SELECT COALESCE(SUM(total_tokens), 0)
             FROM model_usage_events
             WHERE session_id = ?1 AND source = 'model_api_usage'",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(Into::into)
    }

    pub fn model_usage_total_since(&self, since_ms: i64) -> StorageResult<i64> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.query_row(
            "SELECT COALESCE(SUM(total_tokens), 0)
             FROM model_usage_events
             WHERE created_at >= ?1 AND source = 'model_api_usage'",
            params![since_ms.max(0)],
            |row| row.get::<_, i64>(0),
        )
        .map_err(Into::into)
    }

    pub fn create_agent_run(
        &self,
        id: &str,
        session_id: Option<&str>,
        permission_mode: &str,
    ) -> StorageResult<AgentRunRecord> {
        let run_id = compact_text(id.trim(), 96);
        if run_id.is_empty() {
            return Err(StorageError::Validation(
                "agent run id is empty".to_string(),
            ));
        }
        let session_id = session_id.and_then(|value| {
            let trimmed = compact_text(value.trim(), 96);
            (!trimmed.is_empty()).then_some(trimmed)
        });
        if let Some(session_id) = session_id.as_deref() {
            self.ensure_session_exists(session_id)?;
        }

        let now = now_ms();
        let record = AgentRunRecord {
            id: run_id,
            session_id,
            status: "running".to_string(),
            permission_mode: normalize_agent_permission_mode(permission_mode),
            created_at: now,
            updated_at: now,
            finished_at: None,
            error: None,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT OR REPLACE INTO agent_runs(
                id, session_id, status, permission_mode, created_at, updated_at, finished_at, error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &record.id,
                record.session_id.as_deref(),
                &record.status,
                &record.permission_mode,
                record.created_at,
                record.updated_at,
                record.finished_at,
                record.error.as_deref(),
            ],
        )?;
        Ok(record)
    }

    pub fn update_agent_run_status(
        &self,
        run_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> StorageResult<()> {
        let status = normalize_agent_run_status(status);
        let now = now_ms();
        let finished_at =
            matches!(status.as_str(), "finished" | "failed" | "cancelled").then_some(now);
        let error = error
            .map(|value| compact_text(value.trim(), 500))
            .filter(|value| !value.is_empty());
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE agent_runs
             SET status = ?1, updated_at = ?2, finished_at = COALESCE(?3, finished_at), error = ?4
             WHERE id = ?5",
            params![status, now, finished_at, error.as_deref(), run_id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(run_id.to_string()));
        }
        Ok(())
    }

    pub fn append_agent_run_step(
        &self,
        run_id: &str,
        step_type: &str,
        status: &str,
        summary: &str,
        input: serde_json::Value,
        output: serde_json::Value,
    ) -> StorageResult<AgentRunStepRecord> {
        let now = now_ms();
        let step_type = normalize_agent_step_type(step_type);
        let status = normalize_agent_step_status(status);
        let finished_at =
            matches!(status.as_str(), "finished" | "failed" | "cancelled").then_some(now);
        let input = if input.is_null() { json!({}) } else { input };
        let output = if output.is_null() { json!({}) } else { output };
        // P0-1: redact secrets before they are persisted to the run-step log.
        let mask_log = |value: serde_json::Value| -> serde_json::Value {
            let serialized = serde_json::to_string(&value).unwrap_or_default();
            let masked = crate::tools::secret_scan::scan(
                &serialized,
                crate::tools::secret_scan::SecretLocation::Log,
                crate::tools::secret_scan::SecretAction::Masked,
            )
            .text;
            match serde_json::from_str(&masked) {
                Ok(value) => value,
                Err(_) => serde_json::Value::String(masked),
            }
        };
        let input = mask_log(input);
        let output = mask_log(output);
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let step_index: i64 = conn.query_row(
            "SELECT COALESCE(MAX(step_index), 0) + 1 FROM agent_run_steps WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )?;
        let record = AgentRunStepRecord {
            id: format!("ars_{}", Uuid::new_v4()),
            run_id: run_id.to_string(),
            step_index,
            step_type,
            status,
            summary: compact_text(summary.trim(), 500),
            input,
            output,
            created_at: now,
            finished_at,
        };
        conn.execute(
            "INSERT INTO agent_run_steps(
                id, run_id, step_index, step_type, status, summary,
                input_json, output_json, created_at, finished_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &record.id,
                &record.run_id,
                record.step_index,
                &record.step_type,
                &record.status,
                &record.summary,
                serde_json::to_string(&record.input)?,
                serde_json::to_string(&record.output)?,
                record.created_at,
                record.finished_at,
            ],
        )?;
        conn.execute(
            "UPDATE agent_runs SET updated_at = ?1 WHERE id = ?2",
            params![now, run_id],
        )?;
        Ok(record)
    }

    pub fn finish_latest_agent_tool_call_step(
        &self,
        run_id: &str,
        output: serde_json::Value,
    ) -> StorageResult<bool> {
        let now = now_ms();
        let output = if output.is_null() { json!({}) } else { output };
        // P0-1: redact secrets before persisting the tool-call output to the log.
        let output_json = crate::tools::secret_scan::scan(
            &serde_json::to_string(&output)?,
            crate::tools::secret_scan::SecretLocation::Log,
            crate::tools::secret_scan::SecretAction::Masked,
        )
        .text;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE agent_run_steps
             SET status = 'finished', output_json = ?1, finished_at = ?2
             WHERE id = (
                SELECT id
                FROM agent_run_steps
                WHERE run_id = ?3 AND step_type = 'tool_call' AND status = 'running'
                ORDER BY step_index DESC
                LIMIT 1
             )",
            params![output_json, now, run_id],
        )?;
        if changed > 0 {
            conn.execute(
                "UPDATE agent_runs SET updated_at = ?1 WHERE id = ?2",
                params![now, run_id],
            )?;
        }
        Ok(changed > 0)
    }

    pub fn mark_interrupted_agent_runs(&self) -> StorageResult<usize> {
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        // P1-1: capture the runs left `running` before reconciling, so we can drop
        // a visible "interrupted" marker step on each timeline afterwards
        // (acceptance: 时间线能看出上次异常中断，而不仅是 run.error 字段).
        let interrupted_run_ids: Vec<String> = {
            // P1-2: `paused` runs are an in-memory hold (the suspended future + pause
            // handle live in process). After a restart they cannot resume, so they are
            // interrupted just like `running` and must be reconciled together.
            let mut stmt =
                conn.prepare("SELECT id FROM agent_runs WHERE status IN ('running', 'paused')")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            collect_rows(rows)?
        };
        let changed = conn.execute(
            "UPDATE agent_runs
             SET status = 'cancelled',
                 updated_at = ?1,
                 finished_at = COALESCE(finished_at, ?1),
                 error = COALESCE(error, '应用关闭或运行时重启，任务已中断。')
             WHERE status IN ('running', 'paused')",
            params![now],
        )?;
        conn.execute(
            "UPDATE agent_run_steps
             SET status = 'cancelled',
                 finished_at = COALESCE(finished_at, ?1),
                 output_json = CASE
                    WHEN output_json IS NULL OR output_json = '{}' THEN ?2
                    ELSE output_json
                 END
             WHERE status = 'running'
               AND run_id IN (
                   SELECT id FROM agent_runs
                   WHERE status = 'cancelled'
                     AND error = '应用关闭或运行时重启，任务已中断。'
               )",
            params![
                now,
                serde_json::to_string(&json!({
                    "summary": "应用关闭或运行时重启，步骤已中断。"
                }))?
            ],
        )?;
        conn.execute(
            "UPDATE agent_run_steps
             SET status = CASE
                    WHEN (SELECT status FROM agent_runs WHERE id = agent_run_steps.run_id) = 'finished'
                    THEN 'finished'
                    ELSE 'cancelled'
                 END,
                 finished_at = COALESCE(
                    finished_at,
                    (SELECT COALESCE(finished_at, updated_at) FROM agent_runs WHERE id = agent_run_steps.run_id),
                    ?1
                 ),
                 output_json = CASE
                    WHEN output_json IS NULL OR output_json = '{}' THEN ?2
                    ELSE output_json
                 END
             WHERE status = 'running'
               AND run_id IN (
                   SELECT id FROM agent_runs WHERE status IN ('finished', 'failed', 'cancelled')
               )",
            params![
                now,
                serde_json::to_string(&json!({
                    "summary": "任务已结束，遗留运行步骤已自动收尾。"
                }))?
            ],
        )?;
        // P1-1: append a reconcile marker step to each interrupted run's timeline
        // so the UI shows the abnormal interruption explicitly (not just run.error).
        for run_id in &interrupted_run_ids {
            let step_index: i64 = conn.query_row(
                "SELECT COALESCE(MAX(step_index), 0) + 1 FROM agent_run_steps WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )?;
            let summary = "上次运行被异常中断（应用关闭或运行时重启），本次启动已自动收尾。";
            conn.execute(
                "INSERT INTO agent_run_steps(
                    id, run_id, step_index, step_type, status, summary,
                    input_json, output_json, created_at, finished_at
                 ) VALUES (?1, ?2, ?3, 'event', 'cancelled', ?4, '{}', ?5, ?6, ?6)",
                params![
                    format!("ars_{}", Uuid::new_v4()),
                    run_id,
                    step_index,
                    summary,
                    serde_json::to_string(&json!({
                        "summary": summary,
                        "reconciled": true,
                    }))?,
                    now,
                ],
            )?;
        }
        Ok(changed)
    }

    pub fn recent_agent_runs(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> StorageResult<Vec<AgentRunRecord>> {
        let limit = limit.clamp(1, 200) as i64;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, status, permission_mode, created_at, updated_at, finished_at, error
                 FROM agent_runs
                 WHERE session_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![session_id.trim(), limit], agent_run_from_row)?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, status, permission_mode, created_at, updated_at, finished_at, error
                 FROM agent_runs
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], agent_run_from_row)?;
            collect_rows(rows)
        }
    }

    pub fn get_agent_run(&self, run_id: &str) -> StorageResult<Option<AgentRunRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.query_row(
            "SELECT id, session_id, status, permission_mode, created_at, updated_at, finished_at, error
             FROM agent_runs
             WHERE id = ?1",
            params![run_id],
            agent_run_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_agent_run_steps(&self, run_id: &str) -> StorageResult<Vec<AgentRunStepRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, run_id, step_index, step_type, status, summary,
                    input_json, output_json, created_at, finished_at
             FROM agent_run_steps
             WHERE run_id = ?1
             ORDER BY step_index ASC",
        )?;
        let rows = stmt.query_map(params![run_id], agent_run_step_from_row)?;
        collect_rows(rows)
    }

    /// P1-3/P3-5/OS-2: aggregate a run's step + browser-observation +
    /// tool-audit + usage + verify + permission + plan-change events into one chronologically ordered,
    /// paginated timeline for replay.
    /// The seven run-scoped sources are merged in memory (bounded per run on a local
    /// SQLite DB), then sorted and sliced. `total` is the run's full event count, so
    /// the caller can page through the entire run in order — replay is the whole
    /// run, not the latest N events.
    pub fn get_run_timeline(
        &self,
        run_id: &str,
        limit: i64,
        offset: i64,
    ) -> StorageResult<RunTimeline> {
        let limit_clamped = limit.clamp(1, 1000);
        let offset_clamped = offset.max(0);

        // Run metadata first (this getter locks/unlocks on its own) so the page is
        // self-contained for a replay header.
        let run = self.get_agent_run(run_id)?;

        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut entries: Vec<RunTimelineEntry> = Vec::new();

        {
            let mut stmt = conn.prepare(
                "SELECT id, run_id, step_index, step_type, status, summary,
                        input_json, output_json, created_at, finished_at
                 FROM agent_run_steps WHERE run_id = ?1",
            )?;
            for record in stmt.query_map(params![run_id], agent_run_step_from_row)? {
                let r = record?;
                entries.push(RunTimelineEntry {
                    kind: "step".to_string(),
                    id: r.id.clone(),
                    at: r.created_at,
                    finished_at: r.finished_at,
                    seq: r.step_index,
                    label: r.step_type.clone(),
                    status: Some(r.status.clone()),
                    detail: serde_json::to_value(&r).unwrap_or(serde_json::Value::Null),
                });
            }
        }

        {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, step_index, action, target, status, title,
                        url, screenshot_path, dom_summary_json, action_json, result_json,
                        fingerprint, judge_json, loop_detected, created_at
                 FROM browser_agent_steps WHERE run_id = ?1",
            )?;
            for record in stmt.query_map(params![run_id], browser_agent_step_from_row)? {
                let r = record?;
                entries.push(RunTimelineEntry {
                    kind: "browser".to_string(),
                    id: r.id.clone(),
                    at: r.created_at,
                    finished_at: None,
                    seq: r.step_index,
                    label: r.action.clone(),
                    status: Some(r.status.clone()),
                    detail: serde_json::to_value(&r).unwrap_or(serde_json::Value::Null),
                });
            }
        }

        {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, iteration, tool_call_id, tool_name,
                        permission_mode, policy, status, reason, created_at
                 FROM agent_tool_audit_events WHERE run_id = ?1",
            )?;
            for record in stmt.query_map(params![run_id], agent_tool_audit_from_row)? {
                let r = record?;
                entries.push(RunTimelineEntry {
                    kind: "tool".to_string(),
                    id: r.id.clone(),
                    at: r.created_at,
                    finished_at: None,
                    seq: r.iteration,
                    label: r.tool_name.clone(),
                    status: Some(r.status.clone()),
                    detail: serde_json::to_value(&r).unwrap_or(serde_json::Value::Null),
                });
            }
        }

        {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, iteration, provider, model,
                        input_tokens, output_tokens, total_tokens, source, created_at
                 FROM model_usage_events WHERE run_id = ?1",
            )?;
            for record in stmt.query_map(params![run_id], model_usage_from_row)? {
                let r = record?;
                entries.push(RunTimelineEntry {
                    kind: "usage".to_string(),
                    id: r.id.clone(),
                    at: r.created_at,
                    finished_at: None,
                    seq: r.iteration,
                    label: format!("{}/{}", r.provider, r.model),
                    status: None,
                    detail: serde_json::to_value(&r).unwrap_or(serde_json::Value::Null),
                });
            }
        }

        {
            let mut stmt = conn.prepare(
                "SELECT id, run_id, task_id, kind, command, exit_code, status,
                        stdout_tail, stderr_tail, started_at, finished_at
                 FROM run_task_verifications WHERE run_id = ?1",
            )?;
            for record in stmt.query_map(params![run_id], task_verification_from_row)? {
                let r = record?;
                entries.push(RunTimelineEntry {
                    kind: "verify".to_string(),
                    id: r.id.clone(),
                    at: r.started_at,
                    finished_at: r.finished_at,
                    seq: -1,
                    label: r.kind.clone(),
                    status: Some(r.status.clone()),
                    detail: serde_json::to_value(&r).unwrap_or(serde_json::Value::Null),
                });
            }
        }

        {
            // P0-4 permission decisions belong in the event chain: replay must show
            // 谁批 / 为什么 / 确认还是拒绝, not just the tool call. `status` carries the
            // decision (allowed/needs_confirm/denied); `detail` keeps subject/risk/
            // decided_by/reason intact.
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, iteration, tool_call_id, subject,
                        action, risk, mode, decision, reason, decided_by, created_at
                 FROM permission_decisions WHERE run_id = ?1",
            )?;
            for record in stmt.query_map(params![run_id], permission_decision_from_row)? {
                let r = record?;
                entries.push(RunTimelineEntry {
                    kind: "permission".to_string(),
                    id: r.id.clone(),
                    at: r.created_at,
                    finished_at: None,
                    seq: r.iteration,
                    label: r.action.clone(),
                    status: Some(r.decision.clone()),
                    detail: serde_json::to_value(&r).unwrap_or(serde_json::Value::Null),
                });
            }
        }

        {
            // P3-5: plan mutations are part of the run's decision trail. A replay
            // must show what changed and why, not just the current plan state.
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, actor, action, subject_type,
                        subject_id, reason, before_json, after_json, created_at
                 FROM plan_change_events WHERE run_id = ?1",
            )?;
            for record in stmt.query_map(params![run_id], plan_change_from_row)? {
                let r = record?;
                entries.push(RunTimelineEntry {
                    kind: "plan_change".to_string(),
                    id: r.id.clone(),
                    at: r.created_at,
                    finished_at: None,
                    seq: -1,
                    label: format!("{}/{}", r.subject_type, r.action),
                    status: Some(r.actor.clone()),
                    detail: serde_json::to_value(&r).unwrap_or(serde_json::Value::Null),
                });
            }
        }
        drop(conn);

        let (total, page) =
            paginate_run_timeline(entries, limit_clamped as usize, offset_clamped as usize);

        Ok(RunTimeline {
            run_id: run_id.to_string(),
            run,
            total,
            offset: offset_clamped,
            limit: limit_clamped,
            entries: page,
        })
    }

    pub fn record_browser_agent_step(
        &self,
        payload: RecordBrowserAgentStepPayload,
    ) -> StorageResult<BrowserAgentStepRecord> {
        let session_id = payload
            .session_id
            .and_then(|value| clean_optional_id(Some(&value)));
        if let Some(session_id) = session_id.as_deref() {
            self.ensure_session_exists(session_id)?;
        }
        let run_id = payload
            .run_id
            .and_then(|value| clean_optional_id(Some(&value)));
        let action = normalize_browser_action(&payload.action);
        let target = payload
            .target
            .and_then(|value| clean_optional_id(Some(&value)));
        let status = normalize_browser_agent_status(&payload.status);
        let title = payload
            .title
            .map(|value| compact_text(value.trim(), 300))
            .filter(|value| !value.is_empty());
        let url = payload
            .url
            .map(|value| compact_text(value.trim(), 800))
            .filter(|value| !value.is_empty());
        let screenshot_path = payload
            .screenshot_path
            .map(|value| compact_text(value.trim(), 1000))
            .filter(|value| !value.is_empty());
        let dom_summary = normalize_json_object(payload.dom_summary);
        let action_json = mask_log_value(normalize_json_object(payload.action_json));
        let result_json = mask_log_value(normalize_json_object(payload.result_json));
        let judge = normalize_json_object(payload.judge);
        let fingerprint = compact_text(payload.fingerprint.trim(), 128);
        if fingerprint.is_empty() {
            return Err(StorageError::Validation(
                "browser agent step fingerprint is empty".to_string(),
            ));
        }
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let step_index: i64 = match (run_id.as_deref(), session_id.as_deref()) {
            (Some(run_id), _) => conn.query_row(
                "SELECT COALESCE(MAX(step_index), 0) + 1
                 FROM browser_agent_steps
                 WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )?,
            (None, Some(session_id)) => conn.query_row(
                "SELECT COALESCE(MAX(step_index), 0) + 1
                 FROM browser_agent_steps
                 WHERE run_id IS NULL AND session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )?,
            (None, None) => conn.query_row(
                "SELECT COALESCE(MAX(step_index), 0) + 1
                 FROM browser_agent_steps
                 WHERE run_id IS NULL AND session_id IS NULL",
                [],
                |row| row.get(0),
            )?,
        };
        let record = BrowserAgentStepRecord {
            id: format!("bas_{}", Uuid::new_v4()),
            session_id,
            run_id,
            step_index,
            action,
            target,
            status,
            title,
            url,
            screenshot_path,
            dom_summary,
            action_json,
            result_json,
            fingerprint,
            judge,
            loop_detected: payload.loop_detected,
            created_at: now,
        };
        conn.execute(
            "INSERT INTO browser_agent_steps(
                id, session_id, run_id, step_index, action, target, status, title, url,
                screenshot_path, dom_summary_json, action_json, result_json, fingerprint,
                judge_json, loop_detected, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                &record.id,
                record.session_id.as_deref(),
                record.run_id.as_deref(),
                record.step_index,
                &record.action,
                record.target.as_deref(),
                &record.status,
                record.title.as_deref(),
                record.url.as_deref(),
                record.screenshot_path.as_deref(),
                serde_json::to_string(&record.dom_summary)?,
                serde_json::to_string(&record.action_json)?,
                serde_json::to_string(&record.result_json)?,
                &record.fingerprint,
                serde_json::to_string(&record.judge)?,
                bool_to_i64(record.loop_detected),
                record.created_at,
            ],
        )?;
        Ok(record)
    }

    pub fn list_browser_agent_steps(
        &self,
        run_id: Option<&str>,
        session_id: Option<&str>,
        limit: i64,
    ) -> StorageResult<Vec<BrowserAgentStepRecord>> {
        let limit = limit.clamp(1, 500);
        let run_id = run_id.and_then(|value| clean_optional_id(Some(value)));
        let session_id = session_id.and_then(|value| clean_optional_id(Some(value)));
        let conn = self.conn.lock().expect("local db mutex poisoned");
        if let Some(run_id) = run_id.as_deref() {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, step_index, action, target, status, title,
                        url, screenshot_path, dom_summary_json, action_json, result_json,
                        fingerprint, judge_json, loop_detected, created_at
                 FROM (
                    SELECT * FROM browser_agent_steps
                    WHERE run_id = ?1
                    ORDER BY created_at DESC, step_index DESC
                    LIMIT ?2
                 )
                 ORDER BY created_at ASC, step_index ASC",
            )?;
            let rows = stmt.query_map(params![run_id, limit], browser_agent_step_from_row)?;
            return collect_rows(rows);
        }
        if let Some(session_id) = session_id.as_deref() {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, step_index, action, target, status, title,
                        url, screenshot_path, dom_summary_json, action_json, result_json,
                        fingerprint, judge_json, loop_detected, created_at
                 FROM (
                    SELECT * FROM browser_agent_steps
                    WHERE session_id = ?1
                    ORDER BY created_at DESC, step_index DESC
                    LIMIT ?2
                 )
                 ORDER BY created_at ASC, step_index ASC",
            )?;
            let rows = stmt.query_map(params![session_id, limit], browser_agent_step_from_row)?;
            return collect_rows(rows);
        }
        let mut stmt = conn.prepare(
            "SELECT id, session_id, run_id, step_index, action, target, status, title,
                    url, screenshot_path, dom_summary_json, action_json, result_json,
                    fingerprint, judge_json, loop_detected, created_at
             FROM (
                SELECT * FROM browser_agent_steps
                ORDER BY created_at DESC, step_index DESC
                LIMIT ?1
             )
             ORDER BY created_at ASC, step_index ASC",
        )?;
        let rows = stmt.query_map(params![limit], browser_agent_step_from_row)?;
        collect_rows(rows)
    }

    pub fn create_agent_graph_run(
        &self,
        payload: CreateAgentGraphRunPayload,
    ) -> StorageResult<AgentGraphRunRecord> {
        let id = payload
            .id
            .and_then(|value| clean_optional_id(Some(&value)))
            .unwrap_or_else(|| format!("agr_{}", Uuid::new_v4()));
        let session_id = payload
            .session_id
            .and_then(|value| clean_optional_id(Some(&value)));
        if let Some(session_id) = session_id.as_deref() {
            self.ensure_session_exists(session_id)?;
        }
        let source_run_id = payload
            .source_run_id
            .and_then(|value| clean_optional_id(Some(&value)));
        let goal = compact_text(payload.goal.trim(), 1000);
        if goal.is_empty() {
            return Err(StorageError::Validation(
                "agent graph goal is empty".to_string(),
            ));
        }
        let now = now_ms();
        let record = AgentGraphRunRecord {
            id,
            session_id,
            source_run_id,
            goal,
            status: "pending".to_string(),
            created_at: now,
            updated_at: now,
            finished_at: None,
            error: None,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO agent_graph_runs(
                id, session_id, source_run_id, goal, status, created_at, updated_at, finished_at, error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                &record.id,
                record.session_id.as_deref(),
                record.source_run_id.as_deref(),
                &record.goal,
                &record.status,
                record.created_at,
                record.updated_at,
                record.finished_at,
                record.error.as_deref(),
            ],
        )?;
        Ok(record)
    }

    pub fn update_agent_graph_run_status(
        &self,
        graph_run_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> StorageResult<AgentGraphRunRecord> {
        let graph_run_id = compact_text(graph_run_id.trim(), 96);
        if graph_run_id.is_empty() {
            return Err(StorageError::Validation(
                "agent graph run id is empty".to_string(),
            ));
        }
        let status = normalize_graph_run_status(status);
        let now = now_ms();
        let finished_at =
            matches!(status.as_str(), "succeeded" | "failed" | "cancelled").then_some(now);
        let error = error
            .map(|value| compact_text(value.trim(), 500))
            .filter(|value| !value.is_empty());
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE agent_graph_runs
             SET status = ?1, updated_at = ?2, finished_at = COALESCE(?3, finished_at), error = ?4
             WHERE id = ?5",
            params![status, now, finished_at, error.as_deref(), graph_run_id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(graph_run_id));
        }
        self.get_agent_graph_run_locked(&conn, &graph_run_id)?
            .ok_or(StorageError::NotFound(graph_run_id))
    }

    pub fn get_agent_graph_run(
        &self,
        graph_run_id: &str,
    ) -> StorageResult<Option<AgentGraphRunRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        self.get_agent_graph_run_locked(&conn, graph_run_id)
    }

    fn get_agent_graph_run_locked(
        &self,
        conn: &Connection,
        graph_run_id: &str,
    ) -> StorageResult<Option<AgentGraphRunRecord>> {
        conn.query_row(
            "SELECT id, session_id, source_run_id, goal, status, created_at, updated_at, finished_at, error
             FROM agent_graph_runs
             WHERE id = ?1",
            params![graph_run_id],
            agent_graph_run_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn create_agent_graph_node(
        &self,
        payload: CreateAgentGraphNodePayload,
    ) -> StorageResult<AgentGraphNodeRecord> {
        let graph_run_id = compact_text(payload.graph_run_id.trim(), 96);
        let node_key = compact_text(payload.node_key.trim(), 160);
        if graph_run_id.is_empty() || node_key.is_empty() {
            return Err(StorageError::Validation(
                "agent graph node requires graphRunId and nodeKey".to_string(),
            ));
        }
        let now = now_ms();
        let record = AgentGraphNodeRecord {
            id: payload
                .id
                .and_then(|value| clean_optional_id(Some(&value)))
                .unwrap_or_else(|| format!("agn_{}", Uuid::new_v4())),
            graph_run_id,
            node_key,
            kind: normalize_graph_node_kind(&payload.kind),
            title: compact_text(payload.title.trim(), 300),
            status: "pending".to_string(),
            attempt: 0,
            max_attempts: payload.max_attempts.unwrap_or(1).clamp(1, 10),
            input: normalize_json_object(payload.input),
            output: json!({}),
            error: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
        };
        if record.title.is_empty() {
            return Err(StorageError::Validation(
                "agent graph node title is empty".to_string(),
            ));
        }
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO agent_graph_nodes(
                id, graph_run_id, node_key, kind, title, status, attempt, max_attempts,
                input_json, output_json, error, created_at, updated_at, started_at, finished_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                &record.id,
                &record.graph_run_id,
                &record.node_key,
                &record.kind,
                &record.title,
                &record.status,
                record.attempt,
                record.max_attempts,
                serde_json::to_string(&record.input)?,
                serde_json::to_string(&record.output)?,
                record.error.as_deref(),
                record.created_at,
                record.updated_at,
                record.started_at,
                record.finished_at,
            ],
        )?;
        Ok(record)
    }

    pub fn start_agent_graph_node(&self, node_id: &str) -> StorageResult<AgentGraphNodeRecord> {
        let node_id = compact_text(node_id.trim(), 96);
        if node_id.is_empty() {
            return Err(StorageError::Validation(
                "agent graph node id is empty".to_string(),
            ));
        }
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE agent_graph_nodes
             SET status = 'running',
                 attempt = attempt + 1,
                 updated_at = ?1,
                 started_at = COALESCE(started_at, ?1),
                 finished_at = NULL,
                 error = NULL
             WHERE id = ?2",
            params![now, node_id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(node_id));
        }
        self.get_agent_graph_node_locked(&conn, &node_id)?
            .ok_or(StorageError::NotFound(node_id))
    }

    pub fn finish_agent_graph_node(
        &self,
        node_id: &str,
        status: &str,
        output: serde_json::Value,
        error: Option<&str>,
    ) -> StorageResult<AgentGraphNodeRecord> {
        let node_id = compact_text(node_id.trim(), 96);
        if node_id.is_empty() {
            return Err(StorageError::Validation(
                "agent graph node id is empty".to_string(),
            ));
        }
        let status = normalize_graph_node_status(status);
        let now = now_ms();
        let finished_at = matches!(
            status.as_str(),
            "succeeded" | "failed" | "skipped" | "blocked"
        )
        .then_some(now);
        let output = mask_log_value(normalize_json_object(output));
        let error = error
            .map(|value| compact_text(value.trim(), 500))
            .filter(|value| !value.is_empty());
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE agent_graph_nodes
             SET status = ?1, output_json = ?2, error = ?3, updated_at = ?4, finished_at = ?5
             WHERE id = ?6",
            params![
                status,
                serde_json::to_string(&output)?,
                error.as_deref(),
                now,
                finished_at,
                node_id
            ],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(node_id));
        }
        self.get_agent_graph_node_locked(&conn, &node_id)?
            .ok_or(StorageError::NotFound(node_id))
    }

    pub fn get_agent_graph_node(
        &self,
        node_id: &str,
    ) -> StorageResult<Option<AgentGraphNodeRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        self.get_agent_graph_node_locked(&conn, node_id)
    }

    fn get_agent_graph_node_locked(
        &self,
        conn: &Connection,
        node_id: &str,
    ) -> StorageResult<Option<AgentGraphNodeRecord>> {
        conn.query_row(
            "SELECT id, graph_run_id, node_key, kind, title, status, attempt, max_attempts,
                    input_json, output_json, error, created_at, updated_at, started_at, finished_at
             FROM agent_graph_nodes
             WHERE id = ?1",
            params![node_id],
            agent_graph_node_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_agent_graph_nodes(
        &self,
        graph_run_id: &str,
    ) -> StorageResult<Vec<AgentGraphNodeRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, graph_run_id, node_key, kind, title, status, attempt, max_attempts,
                    input_json, output_json, error, created_at, updated_at, started_at, finished_at
             FROM agent_graph_nodes
             WHERE graph_run_id = ?1
             ORDER BY created_at ASC, node_key ASC",
        )?;
        let rows = stmt.query_map(params![graph_run_id], agent_graph_node_from_row)?;
        collect_rows(rows)
    }

    pub fn create_agent_graph_edge(
        &self,
        payload: CreateAgentGraphEdgePayload,
    ) -> StorageResult<AgentGraphEdgeRecord> {
        let graph_run_id = compact_text(payload.graph_run_id.trim(), 96);
        let from_node_id = compact_text(payload.from_node_id.trim(), 96);
        let to_node_id = compact_text(payload.to_node_id.trim(), 96);
        if graph_run_id.is_empty() || from_node_id.is_empty() || to_node_id.is_empty() {
            return Err(StorageError::Validation(
                "agent graph edge requires graphRunId/fromNodeId/toNodeId".to_string(),
            ));
        }
        if from_node_id == to_node_id {
            return Err(StorageError::Validation(
                "agent graph edge cannot point to the same node".to_string(),
            ));
        }
        let now = now_ms();
        let record = AgentGraphEdgeRecord {
            id: payload
                .id
                .and_then(|value| clean_optional_id(Some(&value)))
                .unwrap_or_else(|| format!("age_{}", Uuid::new_v4())),
            graph_run_id,
            from_node_id,
            to_node_id,
            condition: payload
                .condition
                .map(|value| compact_text(value.trim(), 300))
                .filter(|value| !value.is_empty()),
            created_at: now,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO agent_graph_edges(
                id, graph_run_id, from_node_id, to_node_id, condition, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                &record.id,
                &record.graph_run_id,
                &record.from_node_id,
                &record.to_node_id,
                record.condition.as_deref(),
                record.created_at,
            ],
        )?;
        Ok(record)
    }

    pub fn list_agent_graph_edges(
        &self,
        graph_run_id: &str,
    ) -> StorageResult<Vec<AgentGraphEdgeRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, graph_run_id, from_node_id, to_node_id, condition, created_at
             FROM agent_graph_edges
             WHERE graph_run_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![graph_run_id], agent_graph_edge_from_row)?;
        collect_rows(rows)
    }

    pub fn record_agent_graph_checkpoint(
        &self,
        graph_run_id: &str,
        node_id: Option<&str>,
        state: serde_json::Value,
    ) -> StorageResult<AgentGraphCheckpointRecord> {
        let graph_run_id = compact_text(graph_run_id.trim(), 96);
        if graph_run_id.is_empty() {
            return Err(StorageError::Validation(
                "agent graph checkpoint requires graphRunId".to_string(),
            ));
        }
        let node_id = node_id.and_then(|value| clean_optional_id(Some(value)));
        let state = mask_log_value(normalize_json_object(state));
        let record = AgentGraphCheckpointRecord {
            id: format!("agc_{}", Uuid::new_v4()),
            graph_run_id,
            node_id,
            state,
            created_at: now_ms(),
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO agent_graph_checkpoints(id, graph_run_id, node_id, state_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                &record.id,
                &record.graph_run_id,
                record.node_id.as_deref(),
                serde_json::to_string(&record.state)?,
                record.created_at,
            ],
        )?;
        Ok(record)
    }

    pub fn list_agent_graph_checkpoints(
        &self,
        graph_run_id: &str,
        limit: i64,
    ) -> StorageResult<Vec<AgentGraphCheckpointRecord>> {
        let limit = limit.clamp(1, 1000);
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, graph_run_id, node_id, state_json, created_at
             FROM (
                SELECT * FROM agent_graph_checkpoints
                WHERE graph_run_id = ?1
                ORDER BY created_at DESC, id DESC
                LIMIT ?2
             )
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(
            params![graph_run_id, limit],
            agent_graph_checkpoint_from_row,
        )?;
        collect_rows(rows)
    }

    pub fn get_agent_graph_snapshot(
        &self,
        graph_run_id: &str,
    ) -> StorageResult<AgentGraphSnapshot> {
        let run = self
            .get_agent_graph_run(graph_run_id)?
            .ok_or_else(|| StorageError::NotFound(graph_run_id.to_string()))?;
        let nodes = self.list_agent_graph_nodes(graph_run_id)?;
        let edges = self.list_agent_graph_edges(graph_run_id)?;
        let checkpoints = self.list_agent_graph_checkpoints(graph_run_id, 1000)?;
        Ok(AgentGraphSnapshot {
            run,
            nodes,
            edges,
            checkpoints,
        })
    }

    pub fn create_team_run(&self, payload: CreateTeamRunPayload) -> StorageResult<TeamRunRecord> {
        let id = payload
            .id
            .and_then(|value| clean_optional_id(Some(&value)))
            .unwrap_or_else(|| format!("team_{}", Uuid::new_v4()));
        let session_id = payload
            .session_id
            .and_then(|value| clean_optional_id(Some(&value)));
        if let Some(session_id) = session_id.as_deref() {
            self.ensure_session_exists(session_id)?;
        }
        let source_run_id = payload
            .source_run_id
            .and_then(|value| clean_optional_id(Some(&value)));
        let goal = compact_text(payload.goal.trim(), 1000);
        if goal.is_empty() {
            return Err(StorageError::Validation("team goal is empty".to_string()));
        }
        let now = now_ms();
        let record = TeamRunRecord {
            id,
            session_id,
            source_run_id,
            goal,
            status: "running".to_string(),
            max_rounds: payload.max_rounds.unwrap_or(12).clamp(1, 100),
            termination_reason: None,
            created_at: now,
            updated_at: now,
            finished_at: None,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO team_runs(
                id, session_id, source_run_id, goal, status, max_rounds,
                termination_reason, created_at, updated_at, finished_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &record.id,
                record.session_id.as_deref(),
                record.source_run_id.as_deref(),
                &record.goal,
                &record.status,
                record.max_rounds,
                record.termination_reason.as_deref(),
                record.created_at,
                record.updated_at,
                record.finished_at,
            ],
        )?;
        Ok(record)
    }

    pub fn update_team_run_status(
        &self,
        team_run_id: &str,
        status: &str,
        termination_reason: Option<&str>,
    ) -> StorageResult<TeamRunRecord> {
        let team_run_id = compact_text(team_run_id.trim(), 96);
        if team_run_id.is_empty() {
            return Err(StorageError::Validation("team run id is empty".to_string()));
        }
        let status = normalize_team_run_status(status);
        let now = now_ms();
        let finished_at =
            matches!(status.as_str(), "completed" | "failed" | "cancelled").then_some(now);
        let termination_reason = termination_reason
            .map(|value| compact_text(value.trim(), 500))
            .filter(|value| !value.is_empty());
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE team_runs
             SET status = ?1, termination_reason = ?2, updated_at = ?3, finished_at = COALESCE(?4, finished_at)
             WHERE id = ?5",
            params![
                status,
                termination_reason.as_deref(),
                now,
                finished_at,
                team_run_id
            ],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(team_run_id));
        }
        self.get_team_run_locked(&conn, &team_run_id)?
            .ok_or(StorageError::NotFound(team_run_id))
    }

    fn get_team_run_locked(
        &self,
        conn: &Connection,
        team_run_id: &str,
    ) -> StorageResult<Option<TeamRunRecord>> {
        conn.query_row(
            "SELECT id, session_id, source_run_id, goal, status, max_rounds,
                    termination_reason, created_at, updated_at, finished_at
             FROM team_runs
             WHERE id = ?1",
            params![team_run_id],
            team_run_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn add_team_participant(
        &self,
        payload: CreateTeamParticipantPayload,
    ) -> StorageResult<TeamParticipantRecord> {
        let team_run_id = compact_text(payload.team_run_id.trim(), 96);
        let name = compact_text(payload.name.trim(), 160);
        if team_run_id.is_empty() || name.is_empty() {
            return Err(StorageError::Validation(
                "team participant requires teamRunId and name".to_string(),
            ));
        }
        let now = now_ms();
        let record = TeamParticipantRecord {
            id: payload
                .id
                .and_then(|value| clean_optional_id(Some(&value)))
                .unwrap_or_else(|| format!("tp_{}", Uuid::new_v4())),
            team_run_id,
            name,
            role: normalize_team_role(&payload.role),
            model: payload
                .model
                .map(|value| compact_text(value.trim(), 160))
                .filter(|value| !value.is_empty()),
            tool_scope: normalize_json_object(payload.tool_scope),
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO team_participants(
                id, team_run_id, name, role, model, tool_scope_json, status, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                &record.id,
                &record.team_run_id,
                &record.name,
                &record.role,
                record.model.as_deref(),
                serde_json::to_string(&record.tool_scope)?,
                &record.status,
                record.created_at,
                record.updated_at,
            ],
        )?;
        Ok(record)
    }

    pub fn append_team_message(
        &self,
        payload: AppendTeamMessagePayload,
    ) -> StorageResult<TeamMessageRecord> {
        let team_run_id = compact_text(payload.team_run_id.trim(), 96);
        let role = normalize_team_message_role(&payload.role);
        let message_type = normalize_team_message_type(&payload.message_type);
        let content = compact_text(payload.content.trim(), 6000);
        if team_run_id.is_empty() || content.is_empty() {
            return Err(StorageError::Validation(
                "team message requires teamRunId and content".to_string(),
            ));
        }
        let participant_id = payload
            .participant_id
            .and_then(|value| clean_optional_id(Some(&value)));
        let metadata = mask_log_value(normalize_json_object(payload.metadata));
        let record = TeamMessageRecord {
            id: payload
                .id
                .and_then(|value| clean_optional_id(Some(&value)))
                .unwrap_or_else(|| format!("tm_{}", Uuid::new_v4())),
            team_run_id,
            participant_id,
            role,
            message_type,
            content,
            metadata,
            created_at: now_ms(),
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO team_messages(
                id, team_run_id, participant_id, role, message_type, content, metadata_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &record.id,
                &record.team_run_id,
                record.participant_id.as_deref(),
                &record.role,
                &record.message_type,
                &record.content,
                serde_json::to_string(&record.metadata)?,
                record.created_at,
            ],
        )?;
        Ok(record)
    }

    pub fn create_handoff_request(
        &self,
        payload: CreateHandoffRequestPayload,
    ) -> StorageResult<HandoffRequestRecord> {
        let team_run_id = compact_text(payload.team_run_id.trim(), 96);
        let to_participant_id = compact_text(payload.to_participant_id.trim(), 96);
        let reason = compact_text(payload.reason.trim(), 500);
        if team_run_id.is_empty() || to_participant_id.is_empty() || reason.is_empty() {
            return Err(StorageError::Validation(
                "handoff requires teamRunId, target participant, and reason".to_string(),
            ));
        }
        let record = HandoffRequestRecord {
            id: payload
                .id
                .and_then(|value| clean_optional_id(Some(&value)))
                .unwrap_or_else(|| format!("ho_{}", Uuid::new_v4())),
            team_run_id,
            from_participant_id: payload
                .from_participant_id
                .and_then(|value| clean_optional_id(Some(&value))),
            to_participant_id,
            status: "pending".to_string(),
            reason,
            contract: mask_log_value(normalize_json_object(payload.contract)),
            result: json!({}),
            created_at: now_ms(),
            resolved_at: None,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO handoff_requests(
                id, team_run_id, from_participant_id, to_participant_id, status, reason,
                contract_json, result_json, created_at, resolved_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &record.id,
                &record.team_run_id,
                record.from_participant_id.as_deref(),
                &record.to_participant_id,
                &record.status,
                &record.reason,
                serde_json::to_string(&record.contract)?,
                serde_json::to_string(&record.result)?,
                record.created_at,
                record.resolved_at,
            ],
        )?;
        Ok(record)
    }

    pub fn resolve_handoff_request(
        &self,
        handoff_id: &str,
        status: &str,
        result: serde_json::Value,
    ) -> StorageResult<HandoffRequestRecord> {
        let handoff_id = compact_text(handoff_id.trim(), 96);
        let status = normalize_handoff_status(status);
        let resolved_at =
            matches!(status.as_str(), "accepted" | "rejected" | "completed").then_some(now_ms());
        let result = mask_log_value(normalize_json_object(result));
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE handoff_requests
             SET status = ?1, result_json = ?2, resolved_at = COALESCE(?3, resolved_at)
             WHERE id = ?4",
            params![
                status,
                serde_json::to_string(&result)?,
                resolved_at,
                handoff_id,
            ],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(handoff_id));
        }
        conn.query_row(
            "SELECT id, team_run_id, from_participant_id, to_participant_id, status,
                    reason, contract_json, result_json, created_at, resolved_at
             FROM handoff_requests
             WHERE id = ?1",
            params![handoff_id],
            handoff_request_from_row,
        )
        .map_err(Into::into)
    }

    pub fn get_team_run_snapshot(&self, team_run_id: &str) -> StorageResult<TeamRunSnapshot> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let run = self
            .get_team_run_locked(&conn, team_run_id)?
            .ok_or_else(|| StorageError::NotFound(team_run_id.to_string()))?;
        let mut participants_stmt = conn.prepare(
            "SELECT id, team_run_id, name, role, model, tool_scope_json, status, created_at, updated_at
             FROM team_participants
             WHERE team_run_id = ?1
             ORDER BY created_at ASC, name ASC",
        )?;
        let participants = collect_rows(
            participants_stmt.query_map(params![team_run_id], team_participant_from_row)?,
        )?;
        let mut messages_stmt = conn.prepare(
            "SELECT id, team_run_id, participant_id, role, message_type, content, metadata_json, created_at
             FROM team_messages
             WHERE team_run_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let messages =
            collect_rows(messages_stmt.query_map(params![team_run_id], team_message_from_row)?)?;
        let mut handoffs_stmt = conn.prepare(
            "SELECT id, team_run_id, from_participant_id, to_participant_id, status,
                    reason, contract_json, result_json, created_at, resolved_at
             FROM handoff_requests
             WHERE team_run_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let handoffs =
            collect_rows(handoffs_stmt.query_map(params![team_run_id], handoff_request_from_row)?)?;
        Ok(TeamRunSnapshot {
            run,
            participants,
            messages,
            handoffs,
        })
    }

    pub fn add_knowledge_item(
        &self,
        payload: AddKnowledgeItemPayload,
    ) -> StorageResult<KnowledgeItemRecord> {
        let title = compact_text(payload.title.trim(), 300);
        let text = compact_text(payload.text.trim(), 12_000);
        if title.is_empty() || text.is_empty() {
            return Err(StorageError::Validation(
                "knowledge item requires title and text".to_string(),
            ));
        }
        let now = now_ms();
        let scope = normalize_knowledge_scope(&payload.scope);
        let source = compact_text(payload.source.trim(), 300);
        let trust = normalize_knowledge_trust(&payload.trust);
        let confidence = payload.confidence.unwrap_or(0.7).clamp(0.0, 1.0);
        let embedding_ref = payload
            .embedding_ref
            .map(|value| compact_text(value.trim(), 300))
            .filter(|value| !value.is_empty())
            .or_else(|| {
                Some(format!(
                    "lexical:{}",
                    stable_text_hash(&format!("{title}\n{text}"))
                ))
            });
        let record = KnowledgeItemRecord {
            id: payload
                .id
                .and_then(|value| clean_optional_id(Some(&value)))
                .unwrap_or_else(|| format!("ki_{}", Uuid::new_v4())),
            scope,
            source: if source.is_empty() {
                "manual".to_string()
            } else {
                source
            },
            trust,
            title,
            text,
            enabled: true,
            confidence,
            expires_at: payload.expires_at,
            embedding_ref,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO knowledge_items(
                id, scope, source, trust, title, text, enabled, confidence, expires_at,
                embedding_ref, created_at, updated_at, deleted_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                &record.id,
                &record.scope,
                &record.source,
                &record.trust,
                &record.title,
                &record.text,
                bool_to_i64(record.enabled),
                record.confidence,
                record.expires_at,
                record.embedding_ref.as_deref(),
                record.created_at,
                record.updated_at,
                record.deleted_at,
            ],
        )?;
        Ok(record)
    }

    pub fn search_knowledge_items(
        &self,
        query: &str,
        scope: Option<&str>,
        limit: i64,
    ) -> StorageResult<Vec<RetrievalHitRecord>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let query_terms = retrieval_terms(query);
        if query_terms.is_empty() {
            return Ok(Vec::new());
        }
        let now = now_ms();
        let limit = limit.clamp(1, 50) as usize;
        let scope = scope.map(normalize_knowledge_scope);
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut items = if let Some(scope) = scope.as_deref() {
            let mut stmt = conn.prepare(
                "SELECT id, scope, source, trust, title, text, enabled, confidence, expires_at,
                        embedding_ref, created_at, updated_at, deleted_at
                 FROM knowledge_items
                 WHERE enabled = 1
                   AND deleted_at IS NULL
                   AND (expires_at IS NULL OR expires_at > ?1)
                   AND (scope = ?2 OR scope = 'global')
                 ORDER BY updated_at DESC
                 LIMIT 500",
            )?;
            let rows = stmt.query_map(params![now, scope], knowledge_item_from_row)?;
            collect_rows(rows)?
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, scope, source, trust, title, text, enabled, confidence, expires_at,
                        embedding_ref, created_at, updated_at, deleted_at
                 FROM knowledge_items
                 WHERE enabled = 1
                   AND deleted_at IS NULL
                   AND (expires_at IS NULL OR expires_at > ?1)
                 ORDER BY updated_at DESC
                 LIMIT 500",
            )?;
            let rows = stmt.query_map(params![now], knowledge_item_from_row)?;
            collect_rows(rows)?
        };
        let mut hits = items
            .drain(..)
            .filter_map(|item| retrieval_hit_for_item(&query_terms, &item))
            .collect::<Vec<_>>();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| b.created_at.cmp(&a.created_at))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    pub fn apply_knowledge_relevance_feedback(
        &self,
        scope: Option<&str>,
        retrieved_item_ids: &[String],
    ) -> StorageResult<KnowledgeRelevanceFeedbackReport> {
        let scope = scope.map(normalize_knowledge_scope);
        let retrieved = retrieved_item_ids
            .iter()
            .map(|id| compact_text(id.trim(), 96))
            .filter(|id| !id.is_empty())
            .collect::<BTreeSet<_>>();
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let items: Vec<(String, f64)> = if let Some(scope) = scope.as_deref() {
            let mut stmt = conn.prepare(
                "SELECT id, confidence
                 FROM knowledge_items
                 WHERE enabled = 1
                   AND deleted_at IS NULL
                   AND (scope = ?1 OR scope = 'global')",
            )?;
            let rows = stmt.query_map(params![scope], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })?;
            collect_rows(rows)?
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, confidence
                 FROM knowledge_items
                 WHERE enabled = 1
                   AND deleted_at IS NULL",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })?;
            collect_rows(rows)?
        };

        let mut report = KnowledgeRelevanceFeedbackReport::default();
        for (id, confidence) in items {
            if retrieved.contains(&id) {
                let next = (confidence + 0.08).min(1.0);
                conn.execute(
                    "UPDATE knowledge_items
                     SET confidence = ?1, updated_at = ?2
                     WHERE id = ?3",
                    params![next, now, &id],
                )?;
                report.reinforced_item_ids.push(id);
            } else {
                let next = (confidence * 0.92).max(0.0);
                if next < 0.15 {
                    conn.execute(
                        "UPDATE knowledge_items
                         SET enabled = 0, confidence = ?1, deleted_at = ?2, updated_at = ?2
                         WHERE id = ?3",
                        params![next, now, &id],
                    )?;
                    report.soft_deleted_item_ids.push(id);
                } else {
                    conn.execute(
                        "UPDATE knowledge_items
                         SET confidence = ?1, updated_at = ?2
                         WHERE id = ?3",
                        params![next, now, &id],
                    )?;
                    report.decayed_item_ids.push(id);
                }
            }
        }
        Ok(report)
    }

    pub fn delete_knowledge_item(&self, id: &str) -> StorageResult<KnowledgeItemRecord> {
        let id = compact_text(id.trim(), 96);
        if id.is_empty() {
            return Err(StorageError::Validation(
                "knowledge item id is empty".to_string(),
            ));
        }
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE knowledge_items
             SET enabled = 0, deleted_at = ?1, updated_at = ?1
             WHERE id = ?2",
            params![now, id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(id));
        }
        conn.query_row(
            "SELECT id, scope, source, trust, title, text, enabled, confidence, expires_at,
                    embedding_ref, created_at, updated_at, deleted_at
             FROM knowledge_items
             WHERE id = ?1",
            params![id],
            knowledge_item_from_row,
        )
        .map_err(Into::into)
    }

    pub fn create_workspace_lifecycle(
        &self,
        payload: CreateWorkspaceLifecyclePayload,
    ) -> StorageResult<WorkspaceLifecycleRecord> {
        let root_path = normalize_workspace_root(&payload.root_path)?;
        let session_id = payload
            .session_id
            .and_then(|value| clean_optional_id(Some(&value)));
        if let Some(session_id) = session_id.as_deref() {
            self.ensure_session_exists(session_id)?;
        }
        let run_id = payload
            .run_id
            .and_then(|value| clean_optional_id(Some(&value)));
        let sandbox_backend = normalize_sandbox_backend(payload.sandbox_backend.as_deref());
        let fallback_reason = payload
            .fallback_reason
            .map(|value| compact_text(value.trim(), 500))
            .filter(|value| !value.is_empty());
        let setup_script = payload
            .setup_script
            .map(|value| compact_text(value.trim(), 2000))
            .filter(|value| !value.is_empty());
        let now = now_ms();
        let record = WorkspaceLifecycleRecord {
            id: payload
                .id
                .and_then(|value| clean_optional_id(Some(&value)))
                .unwrap_or_else(|| format!("ws_{}", Uuid::new_v4())),
            session_id,
            run_id,
            root_path,
            status: if setup_script.is_some() {
                "created".to_string()
            } else {
                "ready".to_string()
            },
            setup_status: if setup_script.is_some() {
                "pending".to_string()
            } else {
                "skipped".to_string()
            },
            sandbox_backend: sandbox_backend.clone(),
            sandbox_status: if fallback_reason.is_some() {
                "fallback_recorded".to_string()
            } else if sandbox_backend == "local" {
                "boundary_only".to_string()
            } else {
                "ready".to_string()
            },
            fallback_reason,
            setup_script,
            audit: normalize_json_object(payload.audit),
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO workspace_lifecycle_runs(
                id, session_id, run_id, root_path, status, setup_status, sandbox_backend,
                sandbox_status, fallback_reason, setup_script, audit_json, created_at, updated_at, archived_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                &record.id,
                record.session_id.as_deref(),
                record.run_id.as_deref(),
                &record.root_path,
                &record.status,
                &record.setup_status,
                &record.sandbox_backend,
                &record.sandbox_status,
                record.fallback_reason.as_deref(),
                record.setup_script.as_deref(),
                serde_json::to_string(&record.audit)?,
                record.created_at,
                record.updated_at,
                record.archived_at,
            ],
        )?;
        Ok(record)
    }

    pub fn record_workspace_setup_event(
        &self,
        payload: RecordWorkspaceSetupEventPayload,
    ) -> StorageResult<WorkspaceSetupEventRecord> {
        let workspace_id = compact_text(payload.workspace_id.trim(), 96);
        if workspace_id.is_empty() {
            return Err(StorageError::Validation(
                "workspace setup event requires workspaceId".to_string(),
            ));
        }
        let status = normalize_workspace_setup_status(&payload.status);
        let stage = normalize_workspace_stage(&payload.stage);
        let record = WorkspaceSetupEventRecord {
            id: format!("wse_{}", Uuid::new_v4()),
            workspace_id,
            stage: stage.clone(),
            status: status.clone(),
            command: payload
                .command
                .map(|value| compact_text(value.trim(), 1000))
                .filter(|value| !value.is_empty()),
            exit_code: payload.exit_code,
            output_tail: payload
                .output_tail
                .map(|value| tail_text(&value, 4000))
                .unwrap_or_default(),
            reason: compact_text(payload.reason.trim(), 500),
            created_at: now_ms(),
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO workspace_setup_events(
                id, workspace_id, stage, status, command, exit_code, output_tail, reason, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                &record.id,
                &record.workspace_id,
                &record.stage,
                &record.status,
                record.command.as_deref(),
                record.exit_code,
                &record.output_tail,
                &record.reason,
                record.created_at,
            ],
        )?;
        let now = now_ms();
        match status.as_str() {
            "failed" => {
                conn.execute(
                    "UPDATE workspace_lifecycle_runs
                     SET status = 'error', setup_status = ?1, updated_at = ?2
                     WHERE id = ?3",
                    params![status, now, &record.workspace_id],
                )?;
            }
            "succeeded" if stage == "setup" => {
                conn.execute(
                    "UPDATE workspace_lifecycle_runs
                     SET status = 'ready', setup_status = 'succeeded', updated_at = ?1
                     WHERE id = ?2",
                    params![now, &record.workspace_id],
                )?;
            }
            _ => {
                conn.execute(
                    "UPDATE workspace_lifecycle_runs SET updated_at = ?1 WHERE id = ?2",
                    params![now, &record.workspace_id],
                )?;
            }
        }
        Ok(record)
    }

    pub fn get_workspace_lifecycle_snapshot(
        &self,
        workspace_id: &str,
    ) -> StorageResult<WorkspaceLifecycleSnapshot> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let workspace = conn
            .query_row(
                "SELECT id, session_id, run_id, root_path, status, setup_status, sandbox_backend,
                        sandbox_status, fallback_reason, setup_script, audit_json, created_at, updated_at, archived_at
                 FROM workspace_lifecycle_runs
                 WHERE id = ?1",
                params![workspace_id],
                workspace_lifecycle_from_row,
            )
            .optional()?
            .ok_or_else(|| StorageError::NotFound(workspace_id.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, stage, status, command, exit_code, output_tail, reason, created_at
             FROM workspace_setup_events
             WHERE workspace_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let events =
            collect_rows(stmt.query_map(params![workspace_id], workspace_setup_event_from_row)?)?;
        Ok(WorkspaceLifecycleSnapshot { workspace, events })
    }

    pub fn get_workspace_lifecycle_by_run(
        &self,
        run_id: &str,
    ) -> StorageResult<Option<WorkspaceLifecycleRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.query_row(
            "SELECT id, session_id, run_id, root_path, status, setup_status, sandbox_backend,
                    sandbox_status, fallback_reason, setup_script, audit_json, created_at, updated_at, archived_at
             FROM workspace_lifecycle_runs
             WHERE run_id = ?1 AND archived_at IS NULL
             ORDER BY created_at DESC
             LIMIT 1",
            params![run_id],
            workspace_lifecycle_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn persist_eval_run_report(
        &self,
        run: PersistEvalRunPayload,
        cases: Vec<PersistEvalCaseResultPayload>,
        commands: Vec<PersistEvalCommandResultPayload>,
    ) -> StorageResult<EvalRunStorageRecord> {
        if run.id.trim().is_empty() || run.suite_id.trim().is_empty() {
            return Err(StorageError::Validation(
                "eval run id and suite id are required".to_string(),
            ));
        }
        let now = now_ms();
        let report_json = serde_json::to_string(&run.report)?;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO eval_runs(
                id, suite_id, provider, model, status, cwd, passed, started_at,
                finished_at, report_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                suite_id = excluded.suite_id,
                provider = excluded.provider,
                model = excluded.model,
                status = excluded.status,
                cwd = excluded.cwd,
                passed = excluded.passed,
                started_at = excluded.started_at,
                finished_at = excluded.finished_at,
                report_json = excluded.report_json",
            params![
                &run.id,
                &run.suite_id,
                run.provider.as_deref(),
                run.model.as_deref(),
                &run.status,
                &run.cwd,
                bool_to_i64(run.passed),
                run.started_at,
                run.finished_at,
                &report_json,
                now,
            ],
        )?;
        conn.execute(
            "DELETE FROM eval_case_results WHERE eval_run_id = ?1",
            params![&run.id],
        )?;
        conn.execute(
            "DELETE FROM eval_command_results WHERE eval_run_id = ?1",
            params![&run.id],
        )?;
        for case in cases {
            if case.eval_run_id != run.id {
                return Err(StorageError::Validation(
                    "eval case result run id mismatch".to_string(),
                ));
            }
            conn.execute(
                "INSERT INTO eval_case_results(
                    id, eval_run_id, case_id, status, passed, verified,
                    false_completion, blocked, artifact_path, result_json, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    format!("eval_case_{}", Uuid::new_v4()),
                    &case.eval_run_id,
                    &case.case_id,
                    &case.status,
                    bool_to_i64(case.passed),
                    bool_to_i64(case.verified),
                    bool_to_i64(case.false_completion),
                    bool_to_i64(case.blocked),
                    case.artifact_path.as_deref(),
                    serde_json::to_string(&case.result)?,
                    now,
                ],
            )?;
        }
        for command in commands {
            if command.eval_run_id != run.id {
                return Err(StorageError::Validation(
                    "eval command result run id mismatch".to_string(),
                ));
            }
            conn.execute(
                "INSERT INTO eval_command_results(
                    id, eval_run_id, case_id, command, cwd, required, status,
                    exit_code, stdout_tail, stderr_tail, started_at, finished_at,
                    duration_ms, timed_out, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    format!("eval_cmd_{}", Uuid::new_v4()),
                    &command.eval_run_id,
                    &command.case_id,
                    &command.command,
                    &command.cwd,
                    bool_to_i64(command.required),
                    &command.status,
                    command.exit_code,
                    &command.stdout_tail,
                    &command.stderr_tail,
                    command.started_at,
                    command.finished_at,
                    command.duration_ms,
                    bool_to_i64(command.timed_out),
                    now,
                ],
            )?;
        }
        Ok(EvalRunStorageRecord {
            id: run.id,
            suite_id: run.suite_id,
            provider: run.provider,
            model: run.model,
            status: run.status,
            cwd: run.cwd,
            passed: run.passed,
            started_at: run.started_at,
            finished_at: run.finished_at,
            report: run.report,
            created_at: now,
        })
    }

    pub fn get_eval_run_record(&self, run_id: &str) -> StorageResult<Option<EvalRunStorageRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.query_row(
            "SELECT id, suite_id, provider, model, status, cwd, passed, started_at,
                    finished_at, report_json, created_at
             FROM eval_runs
             WHERE id = ?1",
            params![run_id],
            eval_run_storage_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn log_plan_change_event(
        &self,
        session_id: &str,
        run_id: Option<&str>,
        actor: &str,
        action: &str,
        subject_type: &str,
        subject_id: &str,
        reason: &str,
        before: Value,
        after: Value,
    ) -> StorageResult<PlanChangeRecord> {
        self.ensure_session_exists(session_id)?;
        let record = Self::build_plan_change_record(
            session_id,
            run_id,
            actor,
            action,
            subject_type,
            subject_id,
            reason,
            before,
            after,
        )?;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        Self::insert_plan_change_record(&conn, &record)?;
        Ok(record)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_plan_change_record(
        session_id: &str,
        run_id: Option<&str>,
        actor: &str,
        action: &str,
        subject_type: &str,
        subject_id: &str,
        reason: &str,
        before: Value,
        after: Value,
    ) -> StorageResult<PlanChangeRecord> {
        let reason = compact_text(reason.trim(), 500);
        if reason.is_empty() {
            return Err(StorageError::Validation(
                "plan change reason is required".to_string(),
            ));
        }
        Ok(PlanChangeRecord {
            id: format!("planchg_{}", Uuid::new_v4()),
            session_id: session_id.to_string(),
            run_id: clean_optional_id(run_id),
            actor: compact_text(actor.trim(), 80),
            action: normalize_plan_change_action(action),
            subject_type: normalize_plan_change_subject(subject_type),
            subject_id: compact_text(subject_id.trim(), 200),
            reason,
            before,
            after,
            created_at: now_ms(),
        })
    }

    fn insert_plan_change_record(
        conn: &Connection,
        record: &PlanChangeRecord,
    ) -> StorageResult<()> {
        conn.execute(
            "INSERT INTO plan_change_events(
                id, session_id, run_id, actor, action, subject_type, subject_id,
                reason, before_json, after_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                &record.id,
                &record.session_id,
                record.run_id.as_deref(),
                &record.actor,
                &record.action,
                &record.subject_type,
                &record.subject_id,
                &record.reason,
                serde_json::to_string(&record.before)?,
                serde_json::to_string(&record.after)?,
                record.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_plan_change_events(
        &self,
        session_id: &str,
        run_id: Option<&str>,
        limit: i64,
    ) -> StorageResult<Vec<PlanChangeRecord>> {
        let limit = limit.clamp(1, 500);
        let conn = self.conn.lock().expect("local db mutex poisoned");
        if let Some(run_id) = clean_optional_id(run_id) {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, actor, action, subject_type,
                        subject_id, reason, before_json, after_json, created_at
                 FROM plan_change_events
                 WHERE session_id = ?1 AND run_id = ?2
                 ORDER BY created_at DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![session_id, run_id, limit], plan_change_from_row)?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, actor, action, subject_type,
                        subject_id, reason, before_json, after_json, created_at
                 FROM plan_change_events
                 WHERE session_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![session_id, limit], plan_change_from_row)?;
            collect_rows(rows)
        }
    }

    fn validate_plan_task_run_id_for_session(
        conn: &Connection,
        session_id: &str,
        run_id: Option<&str>,
    ) -> StorageResult<Option<String>> {
        let run_id = clean_optional_id(run_id);
        let Some(run_id) = run_id else {
            return Ok(None);
        };
        let run_session_id = conn
            .query_row(
                "SELECT session_id FROM agent_runs WHERE id = ?1",
                params![&run_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .ok_or_else(|| StorageError::NotFound(format!("agent run {run_id}")))?;
        match run_session_id.as_deref() {
            Some(run_session_id) if run_session_id == session_id => Ok(Some(run_id)),
            Some(run_session_id) => Err(StorageError::Validation(format!(
                "plan task run_id {run_id} belongs to session {run_session_id}, not {session_id}"
            ))),
            None => Err(StorageError::Validation(format!(
                "plan task run_id {run_id} is not attached to session {session_id}"
            ))),
        }
    }

    fn plan_task_count(conn: &Connection) -> StorageResult<i64> {
        conn.query_row("SELECT COUNT(*) FROM plan_tasks", [], |row| row.get(0))
            .map_err(Into::into)
    }

    fn plan_task_run_integrity_issues(
        conn: &Connection,
    ) -> StorageResult<Vec<PlanTaskRunIntegrityIssue>> {
        let mut stmt = conn.prepare(
            "SELECT pt.id, pt.session_id, pt.run_id, ar.id, ar.session_id
             FROM plan_tasks pt
             LEFT JOIN agent_runs ar ON ar.id = pt.run_id
             WHERE pt.run_id IS NOT NULL AND TRIM(pt.run_id) <> ''
             ORDER BY pt.created_at ASC, pt.id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let task_id: String = row.get(0)?;
            let session_id: String = row.get(1)?;
            let run_id: String = row.get(2)?;
            let run_exists: Option<String> = row.get(3)?;
            let run_session_id: Option<String> = row.get(4)?;
            let issue = match (run_exists.as_deref(), run_session_id.as_deref()) {
                (None, _) => "missing_run",
                (Some(_), None | Some("")) => "run_without_session",
                (Some(_), Some(run_session_id)) if run_session_id == session_id.as_str() => "ok",
                (Some(_), Some(_)) => "cross_session",
            };
            Ok((
                task_id,
                session_id,
                run_id,
                run_session_id,
                issue.to_string(),
            ))
        })?;
        let mut issues = Vec::new();
        for row in rows {
            let (task_id, session_id, run_id, run_session_id, issue) = row?;
            if issue == "ok" {
                continue;
            }
            issues.push(PlanTaskRunIntegrityIssue {
                task_id,
                session_id,
                run_id,
                issue,
                run_session_id,
            });
        }
        Ok(issues)
    }

    pub fn scan_plan_task_run_integrity(&self) -> StorageResult<PlanTaskRunIntegrityReport> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let scanned_tasks = Self::plan_task_count(&conn)?;
        let issues = Self::plan_task_run_integrity_issues(&conn)?;
        Ok(PlanTaskRunIntegrityReport {
            checked_at: now_ms(),
            scanned_tasks,
            issue_count: issues.len(),
            repaired_count: 0,
            repair_applied: false,
            issues,
        })
    }

    pub fn repair_plan_task_run_integrity(
        &self,
        change_reason: &str,
        actor: &str,
    ) -> StorageResult<PlanTaskRunIntegrityReport> {
        let mut conn = self.conn.lock().expect("local db mutex poisoned");
        let tx = conn.transaction()?;
        let scanned_tasks = Self::plan_task_count(&tx)?;
        let issues = Self::plan_task_run_integrity_issues(&tx)?;
        let now = now_ms();
        let mut repaired_count = 0;
        for issue in &issues {
            let before = tx.query_row(
                "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                        acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
                 FROM plan_tasks WHERE id = ?1",
                params![&issue.task_id],
                plan_task_from_row,
            )?;
            let changed = tx.execute(
                "UPDATE plan_tasks SET run_id = NULL, updated_at = ?1 WHERE id = ?2",
                params![now, &issue.task_id],
            )?;
            if changed == 0 {
                continue;
            }
            repaired_count += 1;
            let mut after = before.clone();
            after.run_id = None;
            after.updated_at = now;
            let plan_change = Self::build_plan_change_record(
                &before.session_id,
                None,
                actor,
                "repair_integrity",
                "plan_task",
                &before.id,
                change_reason,
                serde_json::to_value(&before).unwrap_or(Value::Null),
                serde_json::to_value(&after).unwrap_or(Value::Null),
            )?;
            Self::insert_plan_change_record(&tx, &plan_change)?;
        }
        tx.commit()?;
        Ok(PlanTaskRunIntegrityReport {
            checked_at: now,
            scanned_tasks,
            issue_count: issues.len(),
            repaired_count,
            repair_applied: true,
            issues,
        })
    }

    pub fn create_plan_task(
        &self,
        session_id: &str,
        title: &str,
        parent_id: Option<&str>,
        run_id: Option<&str>,
        source: &str,
    ) -> StorageResult<PlanTaskRecord> {
        self.create_plan_task_full_with_reason(
            session_id,
            title,
            parent_id,
            run_id,
            source,
            None,
            None,
            "创建计划任务。",
            "system",
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_plan_task_full(
        &self,
        session_id: &str,
        title: &str,
        parent_id: Option<&str>,
        run_id: Option<&str>,
        source: &str,
        acceptance_criteria: Option<&serde_json::Value>,
        verify: Option<&serde_json::Value>,
    ) -> StorageResult<PlanTaskRecord> {
        self.create_plan_task_full_with_reason(
            session_id,
            title,
            parent_id,
            run_id,
            source,
            acceptance_criteria,
            verify,
            "创建计划任务。",
            "system",
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_plan_task_full_with_reason(
        &self,
        session_id: &str,
        title: &str,
        parent_id: Option<&str>,
        run_id: Option<&str>,
        source: &str,
        acceptance_criteria: Option<&serde_json::Value>,
        verify: Option<&serde_json::Value>,
        change_reason: &str,
        actor: &str,
    ) -> StorageResult<PlanTaskRecord> {
        self.ensure_session_exists(session_id)?;
        let title = compact_text(title.trim(), 240);
        if title.is_empty() {
            return Err(StorageError::Validation(
                "plan task title is empty".to_string(),
            ));
        }
        let now = now_ms();
        let mut conn = self.conn.lock().expect("local db mutex poisoned");
        let tx = conn.transaction()?;
        let position: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM plan_tasks WHERE session_id = ?1 AND archived_at IS NULL",
            params![session_id],
            |row| row.get(0),
        )?;
        let parent_id = clean_optional_id(parent_id);
        if let Some(parent_id) = parent_id.as_deref() {
            let parent = tx
                .query_row(
                    "SELECT session_id, archived_at FROM plan_tasks WHERE id = ?1",
                    params![parent_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
                )
                .optional()?;
            let Some((parent_session_id, parent_archived_at)) = parent else {
                return Err(StorageError::Validation(format!(
                    "plan task parent not found: {parent_id}"
                )));
            };
            if parent_session_id != session_id {
                return Err(StorageError::Validation(
                    "plan task parent belongs to another session".to_string(),
                ));
            }
            if parent_archived_at.is_some() {
                return Err(StorageError::Validation(
                    "plan task parent is archived".to_string(),
                ));
            }
        }
        let run_id = Self::validate_plan_task_run_id_for_session(&tx, session_id, run_id)?;
        let acceptance_json = acceptance_criteria
            .and_then(|value| (!value.is_null()).then_some(value))
            .map(serde_json::to_string)
            .transpose()?;
        let verify_json = verify
            .and_then(|value| (!value.is_null()).then_some(value))
            .map(serde_json::to_string)
            .transpose()?;
        let record = PlanTaskRecord {
            id: format!("task_{}", Uuid::new_v4()),
            session_id: session_id.to_string(),
            run_id,
            parent_id,
            title,
            status: "pending".to_string(),
            position,
            source: compact_text(source.trim(), 80),
            created_at: now,
            updated_at: now,
            archived_at: None,
            acceptance_criteria: acceptance_criteria.cloned().unwrap_or(json!(null)),
            verify: verify.cloned().unwrap_or(json!(null)),
            evidence: json!(null),
            evidence_status: "none".to_string(),
            active: false,
            blocked_reason: None,
        };
        let plan_change = Self::build_plan_change_record(
            session_id,
            record.run_id.as_deref(),
            actor,
            "create",
            "plan_task",
            &record.id,
            change_reason,
            Value::Null,
            serde_json::to_value(&record).unwrap_or(Value::Null),
        )?;
        tx.execute(
            "INSERT INTO plan_tasks(
                id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, ?12, NULL, 'none', 0, NULL)",
            params![
                &record.id,
                &record.session_id,
                record.run_id.as_deref(),
                record.parent_id.as_deref(),
                &record.title,
                &record.status,
                record.position,
                &record.source,
                record.created_at,
                record.updated_at,
                acceptance_json,
                verify_json,
            ],
        )?;
        Self::insert_plan_change_record(&tx, &plan_change)?;
        tx.commit()?;
        Ok(record)
    }

    pub fn list_plan_tasks(&self, session_id: &str) -> StorageResult<Vec<PlanTaskRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                    acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
             FROM plan_tasks
             WHERE session_id = ?1 AND archived_at IS NULL
             ORDER BY position ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id], plan_task_from_row)?;
        collect_rows(rows)
    }

    pub fn get_plan_task(&self, id: &str) -> StorageResult<PlanTaskRecord> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.query_row(
            "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                    acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
             FROM plan_tasks WHERE id = ?1",
            params![id],
            plan_task_from_row,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(id.to_string()),
            other => other.into(),
        })
    }

    pub fn get_active_plan_task(&self, session_id: &str) -> StorageResult<Option<PlanTaskRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let result = conn.query_row(
            "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                    acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
             FROM plan_tasks WHERE session_id = ?1 AND active = 1 AND archived_at IS NULL
             LIMIT 1",
            params![session_id],
            plan_task_from_row,
        )
        .optional()?;
        Ok(result)
    }

    pub fn set_active_plan_task(
        &self,
        session_id: &str,
        task_id: Option<&str>,
    ) -> StorageResult<Option<PlanTaskRecord>> {
        self.set_active_plan_task_with_reason(
            session_id,
            task_id,
            "切换当前活跃计划任务。",
            "system",
        )
    }

    pub fn set_active_plan_task_with_reason(
        &self,
        session_id: &str,
        task_id: Option<&str>,
        change_reason: &str,
        actor: &str,
    ) -> StorageResult<Option<PlanTaskRecord>> {
        let before = self.get_active_plan_task(session_id)?;
        if before.as_ref().map(|task| task.id.as_str()) == task_id {
            return Ok(before);
        }
        let now = now_ms();
        let mut conn = self.conn.lock().expect("local db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE plan_tasks SET active = 0, updated_at = ?1 WHERE session_id = ?2 AND active = 1",
            params![now, session_id],
        )?;
        let result = if let Some(id) = task_id {
            let changed = tx.execute(
                "UPDATE plan_tasks SET active = 1, updated_at = ?1
                 WHERE id = ?2 AND session_id = ?3 AND archived_at IS NULL",
                params![now, id, session_id],
            )?;
            if changed == 0 {
                return Err(StorageError::NotFound(id.to_string()));
            }
            let task = tx.query_row(
                "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                        acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
                 FROM plan_tasks WHERE id = ?1",
                params![id],
                plan_task_from_row,
            )?;
            Some(task)
        } else {
            None
        };
        let after = result.clone();
        let subject_id = task_id.unwrap_or("none");
        let event_run_id = result
            .as_ref()
            .and_then(|task| task.run_id.as_deref())
            .or_else(|| before.as_ref().and_then(|task| task.run_id.as_deref()));
        let plan_change = Self::build_plan_change_record(
            session_id,
            event_run_id,
            actor,
            "set_active",
            "plan_task",
            subject_id,
            change_reason,
            serde_json::to_value(&before).unwrap_or(Value::Null),
            serde_json::to_value(&after).unwrap_or(Value::Null),
        )?;
        Self::insert_plan_change_record(&tx, &plan_change)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn update_plan_task_status(
        &self,
        id: &str,
        status: &str,
        run_id: Option<&str>,
    ) -> StorageResult<PlanTaskRecord> {
        self.update_plan_task_status_with_reason(id, status, run_id, "更新计划任务状态。", "system")
    }

    pub fn update_plan_task_status_with_reason(
        &self,
        id: &str,
        status: &str,
        run_id: Option<&str>,
        change_reason: &str,
        actor: &str,
    ) -> StorageResult<PlanTaskRecord> {
        let status = normalize_plan_task_status(status);
        let now = now_ms();
        let requested_run_id = clean_optional_id(run_id);
        let mut conn = self.conn.lock().expect("local db mutex poisoned");
        let tx = conn.transaction()?;
        let before = tx
            .query_row(
                "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                        acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
                 FROM plan_tasks WHERE id = ?1",
                params![id],
                plan_task_from_row,
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(id.to_string()),
                other => other.into(),
            })?;
        let run_id = Self::validate_plan_task_run_id_for_session(
            &tx,
            &before.session_id,
            requested_run_id.as_deref(),
        )?;
        let changed = tx.execute(
            "UPDATE plan_tasks
             SET status = ?1,
                 run_id = COALESCE(?2, run_id),
                 updated_at = ?3
             WHERE id = ?4 AND archived_at IS NULL",
            params![status, run_id.as_deref(), now, id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let after = tx.query_row(
            "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                    acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
             FROM plan_tasks WHERE id = ?1",
            params![id],
            plan_task_from_row,
        )?;
        let plan_change = Self::build_plan_change_record(
            &after.session_id,
            after.run_id.as_deref(),
            actor,
            "update_status",
            "plan_task",
            &after.id,
            change_reason,
            serde_json::to_value(&before).unwrap_or(Value::Null),
            serde_json::to_value(&after).unwrap_or(Value::Null),
        )?;
        Self::insert_plan_change_record(&tx, &plan_change)?;
        tx.commit()?;
        Ok(after)
    }

    pub fn update_plan_task_with_reason(
        &self,
        patch: PlanTaskPatch,
        change_reason: &str,
        actor: &str,
    ) -> StorageResult<PlanTaskRecord> {
        let task_id = compact_text(patch.id.trim(), 96);
        if task_id.is_empty() {
            return Err(StorageError::Validation(
                "plan task id is required".to_string(),
            ));
        }
        if patch.clear_parent_id && clean_optional_id(patch.parent_id.as_deref()).is_some() {
            return Err(StorageError::Validation(
                "cannot set and clear plan task parent in the same patch".to_string(),
            ));
        }
        if patch.clear_run_id && clean_optional_id(patch.run_id.as_deref()).is_some() {
            return Err(StorageError::Validation(
                "cannot set and clear plan task run_id in the same patch".to_string(),
            ));
        }
        if patch.clear_acceptance_criteria && patch.acceptance_criteria.is_some() {
            return Err(StorageError::Validation(
                "cannot set and clear acceptance criteria in the same patch".to_string(),
            ));
        }
        if patch.clear_verify && patch.verify.is_some() {
            return Err(StorageError::Validation(
                "cannot set and clear verifier in the same patch".to_string(),
            ));
        }

        let mut conn = self.conn.lock().expect("local db mutex poisoned");
        let tx = conn.transaction()?;
        let before = tx
            .query_row(
                "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                        acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
                 FROM plan_tasks WHERE id = ?1 AND archived_at IS NULL",
                params![&task_id],
                plan_task_from_row,
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(task_id.clone()),
                other => other.into(),
            })?;

        let title = if let Some(title) = patch.title.as_deref() {
            let title = compact_text(title.trim(), 240);
            if title.is_empty() {
                return Err(StorageError::Validation(
                    "plan task title is empty".to_string(),
                ));
            }
            title
        } else {
            before.title.clone()
        };
        let parent_id = if patch.clear_parent_id {
            None
        } else if patch.parent_id.is_some() {
            clean_optional_id(patch.parent_id.as_deref())
        } else {
            before.parent_id.clone()
        };
        if let Some(parent_id) = parent_id.as_deref() {
            let mut current = Some(parent_id.to_string());
            let mut depth = 0;
            while let Some(candidate_id) = current {
                if candidate_id == before.id {
                    return Err(StorageError::Validation(
                        "plan task parent cycle is not allowed".to_string(),
                    ));
                }
                depth += 1;
                if depth > 100 {
                    return Err(StorageError::Validation(
                        "plan task parent chain is too deep".to_string(),
                    ));
                }
                let parent = tx
                    .query_row(
                        "SELECT session_id, parent_id, archived_at FROM plan_tasks WHERE id = ?1",
                        params![&candidate_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, Option<String>>(1)?,
                                row.get::<_, Option<i64>>(2)?,
                            ))
                        },
                    )
                    .optional()?;
                let Some((parent_session_id, parent_parent_id, parent_archived_at)) = parent else {
                    return Err(StorageError::Validation(format!(
                        "plan task parent not found: {candidate_id}"
                    )));
                };
                if parent_session_id != before.session_id {
                    return Err(StorageError::Validation(
                        "plan task parent belongs to another session".to_string(),
                    ));
                }
                if parent_archived_at.is_some() {
                    return Err(StorageError::Validation(
                        "plan task parent is archived".to_string(),
                    ));
                }
                current = parent_parent_id;
            }
        }

        let run_id = if patch.clear_run_id {
            None
        } else if patch.run_id.is_some() {
            clean_optional_id(patch.run_id.as_deref())
        } else {
            before.run_id.clone()
        };
        let run_id = Self::validate_plan_task_run_id_for_session(
            &tx,
            &before.session_id,
            run_id.as_deref(),
        )?;
        let position = patch.position.unwrap_or(before.position).max(0);
        let acceptance_criteria = if patch.clear_acceptance_criteria {
            Value::Null
        } else {
            patch
                .acceptance_criteria
                .unwrap_or_else(|| before.acceptance_criteria.clone())
        };
        let verify = if patch.clear_verify {
            Value::Null
        } else {
            patch.verify.unwrap_or_else(|| before.verify.clone())
        };

        if title == before.title
            && parent_id == before.parent_id
            && run_id == before.run_id
            && position == before.position
            && acceptance_criteria == before.acceptance_criteria
            && verify == before.verify
        {
            return Ok(before);
        }

        let acceptance_json = if acceptance_criteria.is_null() {
            None
        } else {
            Some(serde_json::to_string(&acceptance_criteria)?)
        };
        let verify_json = if verify.is_null() {
            None
        } else {
            Some(serde_json::to_string(&verify)?)
        };
        let now = now_ms();
        let changed = tx.execute(
            "UPDATE plan_tasks
             SET title = ?1,
                 parent_id = ?2,
                 run_id = ?3,
                 position = ?4,
                 acceptance_criteria_json = ?5,
                 verify_json = ?6,
                 updated_at = ?7
             WHERE id = ?8 AND archived_at IS NULL",
            params![
                &title,
                parent_id.as_deref(),
                run_id.as_deref(),
                position,
                acceptance_json.as_deref(),
                verify_json.as_deref(),
                now,
                &before.id,
            ],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(task_id));
        }
        let after = tx.query_row(
            "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                    acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
             FROM plan_tasks WHERE id = ?1",
            params![&before.id],
            plan_task_from_row,
        )?;
        let plan_change = Self::build_plan_change_record(
            &after.session_id,
            after.run_id.as_deref().or(before.run_id.as_deref()),
            actor,
            "update",
            "plan_task",
            &after.id,
            change_reason,
            serde_json::to_value(&before).unwrap_or(Value::Null),
            serde_json::to_value(&after).unwrap_or(Value::Null),
        )?;
        Self::insert_plan_change_record(&tx, &plan_change)?;
        tx.commit()?;
        Ok(after)
    }

    pub fn update_plan_task_evidence(
        &self,
        id: &str,
        evidence: Option<&serde_json::Value>,
        evidence_status: &str,
        blocked_reason: Option<&str>,
    ) -> StorageResult<PlanTaskRecord> {
        self.update_plan_task_evidence_with_reason(
            id,
            evidence,
            evidence_status,
            blocked_reason,
            "更新计划任务证据。",
            "system",
        )
    }

    pub fn update_plan_task_evidence_with_reason(
        &self,
        id: &str,
        evidence: Option<&serde_json::Value>,
        evidence_status: &str,
        blocked_reason: Option<&str>,
        change_reason: &str,
        actor: &str,
    ) -> StorageResult<PlanTaskRecord> {
        let evidence_json = evidence
            .and_then(|value| (!value.is_null()).then_some(value))
            .map(serde_json::to_string)
            .transpose()?;
        let evidence_status = normalize_evidence_status(evidence_status);
        let now = now_ms();
        let blocked = blocked_reason
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| compact_text(s, 500));
        let mut conn = self.conn.lock().expect("local db mutex poisoned");
        let tx = conn.transaction()?;
        let before = tx
            .query_row(
                "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                        acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
                 FROM plan_tasks WHERE id = ?1",
                params![id],
                plan_task_from_row,
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(id.to_string()),
                other => other.into(),
            })?;
        let changed = tx.execute(
            "UPDATE plan_tasks
             SET evidence_json = COALESCE(?1, evidence_json),
                 evidence_status = ?2,
                 blocked_reason = ?3,
                 updated_at = ?4
             WHERE id = ?5 AND archived_at IS NULL",
            params![evidence_json, evidence_status, blocked, now, id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let after = tx.query_row(
            "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                    acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
             FROM plan_tasks WHERE id = ?1",
            params![id],
            plan_task_from_row,
        )?;
        let plan_change = Self::build_plan_change_record(
            &after.session_id,
            after.run_id.as_deref(),
            actor,
            "update_evidence",
            "plan_task",
            &after.id,
            change_reason,
            serde_json::to_value(&before).unwrap_or(Value::Null),
            serde_json::to_value(&after).unwrap_or(Value::Null),
        )?;
        Self::insert_plan_change_record(&tx, &plan_change)?;
        tx.commit()?;
        Ok(after)
    }

    pub fn archive_plan_task(&self, id: &str) -> StorageResult<()> {
        self.archive_plan_task_with_reason(id, "归档计划任务。", "system")
    }

    pub fn archive_plan_task_with_reason(
        &self,
        id: &str,
        change_reason: &str,
        actor: &str,
    ) -> StorageResult<()> {
        let now = now_ms();
        let mut conn = self.conn.lock().expect("local db mutex poisoned");
        let tx = conn.transaction()?;
        let before = tx
            .query_row(
                "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                        acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
                 FROM plan_tasks WHERE id = ?1",
                params![id],
                plan_task_from_row,
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(id.to_string()),
                other => other.into(),
            })?;
        let changed = tx.execute(
            "UPDATE plan_tasks SET archived_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        let mut after = before.clone();
        after.archived_at = Some(now);
        after.updated_at = now;
        let plan_change = Self::build_plan_change_record(
            &before.session_id,
            before.run_id.as_deref(),
            actor,
            "archive",
            "plan_task",
            &before.id,
            change_reason,
            serde_json::to_value(&before).unwrap_or(Value::Null),
            serde_json::to_value(&after).unwrap_or(Value::Null),
        )?;
        Self::insert_plan_change_record(&tx, &plan_change)?;
        tx.commit()?;
        Ok(())
    }

    pub fn record_artifact(&self, payload: RecordArtifactPayload) -> StorageResult<ArtifactRecord> {
        let now = now_ms();
        let record = ArtifactRecord {
            id: format!("artifact_{}", Uuid::new_v4()),
            session_id: payload
                .session_id
                .and_then(|value| clean_optional_id(Some(&value))),
            run_id: payload
                .run_id
                .and_then(|value| clean_optional_id(Some(&value))),
            kind: normalize_artifact_kind(&payload.kind),
            title: compact_text(payload.title.trim(), 160),
            path: payload.path.and_then(|value| {
                let trimmed = compact_text(value.trim(), 1000);
                (!trimmed.is_empty()).then_some(trimmed)
            }),
            operation: compact_text(payload.operation.trim(), 80),
            status: normalize_artifact_status(&payload.status),
            summary: compact_text(payload.summary.trim(), 500),
            metadata: if payload.metadata.is_null() {
                json!({})
            } else {
                payload.metadata
            },
            created_at: now,
            updated_at: now,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO artifacts(
                id, session_id, run_id, kind, title, path, operation, status, summary, metadata, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                &record.id,
                record.session_id.as_deref(),
                record.run_id.as_deref(),
                &record.kind,
                &record.title,
                record.path.as_deref(),
                &record.operation,
                &record.status,
                &record.summary,
                serde_json::to_string(&record.metadata)?,
                record.created_at,
                record.updated_at,
            ],
        )?;
        Ok(record)
    }

    pub fn validate_artifact_scope(
        &self,
        session_id: Option<&str>,
        run_id: Option<&str>,
    ) -> StorageResult<(Option<String>, Option<String>)> {
        let run_id = clean_optional_id(run_id);
        let mut session_id = clean_optional_id(session_id);
        if let Some(run_id) = run_id.as_deref() {
            let run = self
                .get_agent_run(run_id)?
                .ok_or_else(|| StorageError::NotFound(format!("agent run {run_id}")))?;
            if let Some(run_session_id) = run.session_id.as_deref() {
                if let Some(session_id) = session_id.as_deref() {
                    if session_id != run_session_id {
                        return Err(StorageError::Validation(format!(
                            "agent run {run_id} belongs to session {run_session_id}, not {session_id}"
                        )));
                    }
                } else {
                    session_id = Some(run_session_id.to_string());
                }
                self.ensure_session_exists(run_session_id)?;
            } else if let Some(session_id) = session_id.as_deref() {
                return Err(StorageError::Validation(format!(
                    "agent run {run_id} is not attached to session {session_id}"
                )));
            }
        } else if let Some(session_id) = session_id.as_deref() {
            self.ensure_session_exists(session_id)?;
        }
        Ok((session_id, run_id))
    }

    pub fn recent_artifacts(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> StorageResult<Vec<ArtifactRecord>> {
        let limit = limit.clamp(1, 200) as i64;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, kind, title, path, operation, status, summary, metadata, created_at, updated_at
                 FROM artifacts
                 WHERE session_id = ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![session_id.trim(), limit], artifact_from_row)?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, run_id, kind, title, path, operation, status, summary, metadata, created_at, updated_at
                 FROM artifacts
                 ORDER BY updated_at DESC
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], artifact_from_row)?;
            collect_rows(rows)
        }
    }

    fn all_agent_runs(&self) -> StorageResult<Vec<AgentRunRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, status, permission_mode, created_at, updated_at, finished_at, error
             FROM agent_runs ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], agent_run_from_row)?;
        collect_rows(rows)
    }

    fn all_agent_run_steps(&self) -> StorageResult<Vec<AgentRunStepRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, run_id, step_index, step_type, status, summary, input_json, output_json, created_at, finished_at
             FROM agent_run_steps ORDER BY created_at ASC, step_index ASC",
        )?;
        let rows = stmt.query_map([], agent_run_step_from_row)?;
        collect_rows(rows)
    }

    fn all_agent_tool_audit_events(&self) -> StorageResult<Vec<AgentToolAuditRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, run_id, iteration, tool_call_id, tool_name, permission_mode, policy, status, reason, created_at
             FROM agent_tool_audit_events ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], agent_tool_audit_from_row)?;
        collect_rows(rows)
    }

    fn all_model_usage_events(&self) -> StorageResult<Vec<ModelUsageRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, run_id, iteration, provider, model,
                    input_tokens, output_tokens, total_tokens, source, created_at
             FROM model_usage_events ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], model_usage_from_row)?;
        collect_rows(rows)
    }

    fn all_plan_tasks(&self) -> StorageResult<Vec<PlanTaskRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, run_id, parent_id, title, status, position, source, created_at, updated_at, archived_at,
                    acceptance_criteria_json, verify_json, evidence_json, evidence_status, active, blocked_reason
             FROM plan_tasks ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], plan_task_from_row)?;
        collect_rows(rows)
    }

    fn all_plan_change_events(&self) -> StorageResult<Vec<PlanChangeRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, run_id, actor, action, subject_type,
                    subject_id, reason, before_json, after_json, created_at
             FROM plan_change_events ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], plan_change_from_row)?;
        collect_rows(rows)
    }

    pub fn upsert_plugin_package(
        &self,
        mut record: PluginPackageRecord,
    ) -> StorageResult<PluginPackageRecord> {
        record.id = compact_text(record.id.trim(), 120);
        record.name = compact_text(record.name.trim(), 160);
        record.version = compact_text(record.version.trim(), 80);
        record.source = compact_text(record.source.trim(), 240);
        record.description = compact_text(record.description.trim(), 500);
        record.risk = normalize_permission_risk(&record.risk);
        if record.id.is_empty() {
            return Err(StorageError::Validation(
                "plugin package id is empty".to_string(),
            ));
        }
        if record.name.is_empty() {
            return Err(StorageError::Validation(
                "plugin package name is empty".to_string(),
            ));
        }
        if record.version.is_empty() {
            record.version = "0.1.0".to_string();
        }
        if record.source.is_empty() {
            record.source = "local".to_string();
        }
        if !record.permissions.is_array() {
            record.permissions = json!([]);
        }
        if !record.capabilities.is_array() {
            return Err(StorageError::Validation(
                "plugin package capabilities must be an array".to_string(),
            ));
        }
        if !record.manifest.is_object() {
            record.manifest = json!({});
        }
        let now = now_ms();
        if record.installed_at <= 0 {
            record.installed_at = now;
        }
        record.updated_at = now;
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO plugin_packages(
                id, name, version, source, description, trusted, enabled, risk,
                permissions_json, capabilities_json, manifest_json, installed_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                version = excluded.version,
                source = excluded.source,
                description = excluded.description,
                trusted = excluded.trusted,
                enabled = excluded.enabled,
                risk = excluded.risk,
                permissions_json = excluded.permissions_json,
                capabilities_json = excluded.capabilities_json,
                manifest_json = excluded.manifest_json,
                updated_at = excluded.updated_at",
            params![
                &record.id,
                &record.name,
                &record.version,
                &record.source,
                &record.description,
                bool_to_i64(record.trusted),
                bool_to_i64(record.enabled),
                &record.risk,
                serde_json::to_string(&record.permissions)?,
                serde_json::to_string(&record.capabilities)?,
                serde_json::to_string(&record.manifest)?,
                record.installed_at,
                record.updated_at,
            ],
        )?;
        Ok(record)
    }

    pub fn list_plugin_packages(&self) -> StorageResult<Vec<PluginPackageRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, version, source, description, trusted, enabled, risk,
                    permissions_json, capabilities_json, manifest_json, installed_at, updated_at
             FROM plugin_packages ORDER BY updated_at DESC, id ASC",
        )?;
        let rows = stmt.query_map([], plugin_package_from_row)?;
        collect_rows(rows)
    }

    pub fn list_enabled_plugin_packages(&self) -> StorageResult<Vec<PluginPackageRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, version, source, description, trusted, enabled, risk,
                    permissions_json, capabilities_json, manifest_json, installed_at, updated_at
             FROM plugin_packages WHERE enabled = 1 ORDER BY updated_at DESC, id ASC",
        )?;
        let rows = stmt.query_map([], plugin_package_from_row)?;
        collect_rows(rows)
    }

    pub fn get_plugin_package(&self, id: &str) -> StorageResult<PluginPackageRecord> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.query_row(
            "SELECT id, name, version, source, description, trusted, enabled, risk,
                    permissions_json, capabilities_json, manifest_json, installed_at, updated_at
             FROM plugin_packages WHERE id = ?1",
            params![id],
            plugin_package_from_row,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(id.to_string()),
            other => other.into(),
        })
    }

    pub fn set_plugin_package_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> StorageResult<PluginPackageRecord> {
        let now = now_ms();
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE plugin_packages SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            params![bool_to_i64(enabled), now, id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        conn.query_row(
            "SELECT id, name, version, source, description, trusted, enabled, risk,
                    permissions_json, capabilities_json, manifest_json, installed_at, updated_at
             FROM plugin_packages WHERE id = ?1",
            params![id],
            plugin_package_from_row,
        )
        .map_err(Into::into)
    }

    pub fn log_plugin_capability_event(
        &self,
        payload: LogPluginCapabilityEventPayload,
    ) -> StorageResult<PluginCapabilityEventRecord> {
        let record = PluginCapabilityEventRecord {
            id: format!("pluginevt_{}", Uuid::new_v4()),
            plugin_id: compact_text(payload.plugin_id.trim(), 120),
            capability_id: compact_text(payload.capability_id.trim(), 120),
            action: normalize_plugin_event_action(&payload.action),
            status: normalize_plugin_event_status(&payload.status),
            risk: normalize_permission_risk(&payload.risk),
            reason: compact_text(payload.reason.trim(), 500),
            input: payload.input,
            output: payload.output,
            created_at: now_ms(),
        };
        if record.plugin_id.is_empty() || record.capability_id.is_empty() {
            return Err(StorageError::Validation(
                "plugin event requires plugin_id and capability_id".to_string(),
            ));
        }
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO plugin_capability_events(
                id, plugin_id, capability_id, action, status, risk, reason,
                input_json, output_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &record.id,
                &record.plugin_id,
                &record.capability_id,
                &record.action,
                &record.status,
                &record.risk,
                &record.reason,
                serde_json::to_string(&record.input)?,
                serde_json::to_string(&record.output)?,
                record.created_at,
            ],
        )?;
        Ok(record)
    }

    pub fn list_plugin_capability_events(
        &self,
        plugin_id: Option<&str>,
        limit: i64,
    ) -> StorageResult<Vec<PluginCapabilityEventRecord>> {
        let limit = limit.clamp(1, 500);
        let conn = self.conn.lock().expect("local db mutex poisoned");
        if let Some(plugin_id) = clean_optional_id(plugin_id) {
            let mut stmt = conn.prepare(
                "SELECT id, plugin_id, capability_id, action, status, risk, reason,
                        input_json, output_json, created_at
                 FROM plugin_capability_events
                 WHERE plugin_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
            let rows =
                stmt.query_map(params![plugin_id, limit], plugin_capability_event_from_row)?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, plugin_id, capability_id, action, status, risk, reason,
                        input_json, output_json, created_at
                 FROM plugin_capability_events
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], plugin_capability_event_from_row)?;
            collect_rows(rows)
        }
    }

    fn all_plugin_packages(&self) -> StorageResult<Vec<PluginPackageRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, version, source, description, trusted, enabled, risk,
                    permissions_json, capabilities_json, manifest_json, installed_at, updated_at
             FROM plugin_packages ORDER BY installed_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], plugin_package_from_row)?;
        collect_rows(rows)
    }

    fn all_plugin_capability_events(&self) -> StorageResult<Vec<PluginCapabilityEventRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, plugin_id, capability_id, action, status, risk, reason,
                    input_json, output_json, created_at
             FROM plugin_capability_events ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], plugin_capability_event_from_row)?;
        collect_rows(rows)
    }

    pub fn create_run_plan(
        &self,
        session_id: &str,
        goal: &str,
        run_id: Option<&str>,
        acceptance_criteria: Option<&serde_json::Value>,
        observable_outcome: Option<&str>,
        non_goals: Option<&serde_json::Value>,
    ) -> StorageResult<RunPlanRecord> {
        self.create_run_plan_with_reason(
            session_id,
            goal,
            run_id,
            acceptance_criteria,
            observable_outcome,
            non_goals,
            "创建运行计划。",
            "system",
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_run_plan_with_reason(
        &self,
        session_id: &str,
        goal: &str,
        run_id: Option<&str>,
        acceptance_criteria: Option<&serde_json::Value>,
        observable_outcome: Option<&str>,
        non_goals: Option<&serde_json::Value>,
        change_reason: &str,
        actor: &str,
    ) -> StorageResult<RunPlanRecord> {
        self.ensure_session_exists(session_id)?;
        let goal = compact_text(goal.trim(), 500);
        if goal.is_empty() {
            return Err(StorageError::Validation("plan goal is empty".to_string()));
        }
        let now = now_ms();
        let acceptance_json = acceptance_criteria
            .and_then(|value| (!value.is_null()).then_some(value))
            .map(serde_json::to_string)
            .transpose()?;
        // P3-4: store the observable outcome (a result) separately from the goal
        // text so final audit can judge against it; empty strings collapse to None.
        let observable_outcome = observable_outcome
            .map(|s| compact_text(s.trim(), 500))
            .filter(|s| !s.is_empty());
        let non_goals_value = non_goals
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| json!([]));
        let non_goals_json = non_goals
            .and_then(|value| (!value.is_null()).then_some(value))
            .map(serde_json::to_string)
            .transpose()?;
        let record = RunPlanRecord {
            id: format!("plan_{}", Uuid::new_v4()),
            run_id: clean_optional_id(run_id),
            session_id: session_id.to_string(),
            goal,
            observable_outcome,
            non_goals: non_goals_value,
            acceptance_criteria: acceptance_criteria.cloned().unwrap_or(json!(null)),
            status: "draft".to_string(),
            created_at: now,
            updated_at: now,
        };
        let plan_change = Self::build_plan_change_record(
            session_id,
            record.run_id.as_deref(),
            actor,
            "create",
            "run_plan",
            &record.id,
            change_reason,
            Value::Null,
            serde_json::to_value(&record).unwrap_or(Value::Null),
        )?;
        let mut conn = self.conn.lock().expect("local db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO run_plans(
                id, run_id, session_id, goal, observable_outcome, non_goals_json,
                acceptance_criteria_json, status, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &record.id,
                record.run_id.as_deref(),
                &record.session_id,
                &record.goal,
                record.observable_outcome.as_deref(),
                non_goals_json,
                acceptance_json,
                &record.status,
                record.created_at,
                record.updated_at,
            ],
        )?;
        Self::insert_plan_change_record(&tx, &plan_change)?;
        tx.commit()?;
        Ok(record)
    }

    pub fn list_run_plans(&self, session_id: &str) -> StorageResult<Vec<RunPlanRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, run_id, session_id, goal, observable_outcome, non_goals_json,
                    acceptance_criteria_json, status, created_at, updated_at
             FROM run_plans WHERE session_id = ?1 ORDER BY updated_at DESC, created_at DESC",
        )?;
        let rows = stmt.query_map(params![session_id], run_plan_from_row)?;
        collect_rows(rows)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_task_verification(
        &self,
        task_id: &str,
        run_id: Option<&str>,
        kind: &str,
        command: &str,
        exit_code: Option<i64>,
        status: &str,
        stdout_tail: &str,
        stderr_tail: &str,
        started_at: i64,
        finished_at: Option<i64>,
    ) -> StorageResult<TaskVerificationRecord> {
        let record = TaskVerificationRecord {
            id: format!("verif_{}", Uuid::new_v4()),
            run_id: clean_optional_id(run_id),
            task_id: task_id.to_string(),
            kind: normalize_verification_kind(kind),
            command: compact_text(command.trim(), 4000),
            exit_code,
            status: normalize_verification_status(status),
            stdout_tail: tail_text(stdout_tail, 4000),
            stderr_tail: tail_text(stderr_tail, 4000),
            started_at,
            finished_at,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO run_task_verifications(
                id, run_id, task_id, kind, command, exit_code, status, stdout_tail, stderr_tail, started_at, finished_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                &record.id,
                record.run_id.as_deref(),
                &record.task_id,
                &record.kind,
                &record.command,
                record.exit_code,
                &record.status,
                &record.stdout_tail,
                &record.stderr_tail,
                record.started_at,
                record.finished_at,
            ],
        )?;
        Ok(record)
    }

    pub fn list_task_verifications(
        &self,
        task_id: &str,
    ) -> StorageResult<Vec<TaskVerificationRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, run_id, task_id, kind, command, exit_code, status, stdout_tail, stderr_tail, started_at, finished_at
             FROM run_task_verifications WHERE task_id = ?1 ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map(params![task_id], task_verification_from_row)?;
        collect_rows(rows)
    }

    pub fn list_task_verifications_by_run(
        &self,
        run_id: &str,
    ) -> StorageResult<Vec<TaskVerificationRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, run_id, task_id, kind, command, exit_code, status, stdout_tail, stderr_tail, started_at, finished_at
             FROM run_task_verifications WHERE run_id = ?1 ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map(params![run_id], task_verification_from_row)?;
        collect_rows(rows)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_file_checkpoint(
        &self,
        path: &str,
        run_id: Option<&str>,
        task_id: Option<&str>,
        before_hash: Option<&str>,
        after_hash: Option<&str>,
        before_content: Option<&str>,
        before_blob_path: Option<&str>,
        before_size: i64,
    ) -> StorageResult<FileCheckpointRecord> {
        let now = now_ms();
        let record = FileCheckpointRecord {
            id: format!("ckpt_{}", Uuid::new_v4()),
            run_id: clean_optional_id(run_id),
            task_id: clean_optional_id(task_id),
            path: path.to_string(),
            before_hash: before_hash.map(|s| s.to_string()),
            after_hash: after_hash.map(|s| s.to_string()),
            after_content: None,
            before_content: before_content.map(|s| s.to_string()),
            before_blob_path: before_blob_path.map(|s| s.to_string()),
            before_size,
            created_at: now,
            restored_at: None,
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "INSERT INTO run_file_checkpoints(
                id, run_id, task_id, path, before_hash, after_hash, after_content, before_content, before_blob_path, before_size, created_at, restored_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)",
            params![
                &record.id,
                record.run_id.as_deref(),
                record.task_id.as_deref(),
                &record.path,
                record.before_hash.as_deref(),
                record.after_hash.as_deref(),
                record.after_content.as_deref(),
                record.before_content.as_deref(),
                record.before_blob_path.as_deref(),
                record.before_size,
                record.created_at,
            ],
        )?;
        Ok(record)
    }

    pub fn list_file_checkpoints(&self, task_id: &str) -> StorageResult<Vec<FileCheckpointRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, run_id, task_id, path, before_hash, after_hash, after_content, before_content, before_blob_path, before_size, created_at, restored_at
             FROM run_file_checkpoints WHERE task_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![task_id], file_checkpoint_from_row)?;
        collect_rows(rows)
    }

    pub fn list_file_checkpoints_by_run(
        &self,
        run_id: &str,
    ) -> StorageResult<Vec<FileCheckpointRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, run_id, task_id, path, before_hash, after_hash, after_content, before_content, before_blob_path, before_size, created_at, restored_at
             FROM run_file_checkpoints WHERE run_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![run_id], file_checkpoint_from_row)?;
        collect_rows(rows)
    }

    pub fn set_file_checkpoint_after_hash(
        &self,
        checkpoint_id: &str,
        after_hash: Option<&str>,
    ) -> StorageResult<()> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE run_file_checkpoints SET after_hash = ?1 WHERE id = ?2",
            params![after_hash, checkpoint_id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(checkpoint_id.to_string()));
        }
        Ok(())
    }

    pub fn set_file_checkpoint_after_snapshot(
        &self,
        checkpoint_id: &str,
        after_hash: Option<&str>,
        after_content: Option<&str>,
    ) -> StorageResult<()> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let changed = conn.execute(
            "UPDATE run_file_checkpoints SET after_hash = ?1, after_content = ?2 WHERE id = ?3",
            params![after_hash, after_content, checkpoint_id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(checkpoint_id.to_string()));
        }
        Ok(())
    }

    pub fn mark_file_checkpoint_restored(&self, checkpoint_id: &str) -> StorageResult<()> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "UPDATE run_file_checkpoints SET restored_at = ?1 WHERE id = ?2",
            params![now_ms(), checkpoint_id],
        )?;
        Ok(())
    }

    pub fn delete_file_checkpoint(&self, checkpoint_id: &str) -> StorageResult<()> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.execute(
            "DELETE FROM run_file_checkpoints WHERE id = ?1",
            params![checkpoint_id],
        )?;
        Ok(())
    }

    // ----- provider_capabilities (M5) -----

    pub fn get_provider_capabilities(
        &self,
        provider_id: &str,
        model: &str,
    ) -> StorageResult<Option<ProviderCapabilitiesRow>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        conn.query_row(
            "SELECT provider_id, model, vision, tool_calls, json_mode, max_context, source, updated_at
             FROM provider_capabilities WHERE provider_id = ?1 AND model = ?2",
            params![provider_id, model],
            provider_capabilities_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_provider_capabilities(&self) -> StorageResult<Vec<ProviderCapabilitiesRow>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT provider_id, model, vision, tool_calls, json_mode, max_context, source, updated_at
             FROM provider_capabilities ORDER BY provider_id, model",
        )?;
        let rows = stmt.query_map([], provider_capabilities_from_row)?;
        collect_rows(rows)
    }

    pub fn upsert_provider_capabilities(&self, row: &ProviderCapabilitiesRow) -> StorageResult<()> {
        // T30: capture per-field diff vs existing row, write capability_audit
        // entries, then upsert. Note: we hold conn for the full op (no nested
        // self.* calls) to avoid Mutex re-entrancy (cf. §300 deadlock).
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let existing: Option<ProviderCapabilitiesRow> = conn
            .query_row(
                "SELECT provider_id, model, vision, tool_calls, json_mode, max_context, source, updated_at
                 FROM provider_capabilities WHERE provider_id = ?1 AND model = ?2",
                params![row.provider_id, row.model],
                provider_capabilities_from_row,
            )
            .optional()?;

        let now = now_ms();
        let source_before = existing.as_ref().map(|r| r.source.clone());
        let audit_entries: Vec<(&'static str, String, String)> = if let Some(old) = &existing {
            let mut diffs = Vec::new();
            if old.vision != row.vision {
                diffs.push(("vision", old.vision.to_string(), row.vision.to_string()));
            }
            if old.tool_calls != row.tool_calls {
                diffs.push((
                    "tool_calls",
                    old.tool_calls.to_string(),
                    row.tool_calls.to_string(),
                ));
            }
            if old.json_mode != row.json_mode {
                diffs.push((
                    "json_mode",
                    old.json_mode.to_string(),
                    row.json_mode.to_string(),
                ));
            }
            if old.max_context != row.max_context {
                diffs.push((
                    "max_context",
                    old.max_context.to_string(),
                    row.max_context.to_string(),
                ));
            }
            if old.source != row.source {
                diffs.push(("source", old.source.clone(), row.source.clone()));
            }
            diffs
        } else {
            // First insert — log creation diff for each non-default field.
            vec![("created", "<missing>".to_string(), row.source.clone())]
        };

        conn.execute(
            "INSERT INTO provider_capabilities
                (provider_id, model, vision, tool_calls, json_mode, max_context, source, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(provider_id, model) DO UPDATE SET
                vision = excluded.vision,
                tool_calls = excluded.tool_calls,
                json_mode = excluded.json_mode,
                max_context = excluded.max_context,
                source = excluded.source,
                updated_at = excluded.updated_at",
            params![
                row.provider_id,
                row.model,
                if row.vision { 1_i64 } else { 0_i64 },
                if row.tool_calls { 1_i64 } else { 0_i64 },
                if row.json_mode { 1_i64 } else { 0_i64 },
                row.max_context as i64,
                row.source,
                now,
            ],
        )?;

        for (field, old, new) in audit_entries {
            conn.execute(
                "INSERT INTO capability_audit
                    (provider_id, model, field, old_value, new_value, source_before, source_after, changed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    row.provider_id,
                    row.model,
                    field,
                    old,
                    new,
                    source_before.as_deref().unwrap_or(""),
                    row.source,
                    now,
                ],
            )?;
        }
        Ok(())
    }

    /// T30: read capability audit history. If provider_id is Some, filter to
    /// that provider; if both are Some, also filter to model.
    pub fn list_capability_audit(
        &self,
        provider_id: Option<&str>,
        model: Option<&str>,
        limit: i64,
    ) -> StorageResult<Vec<CapabilityAuditRow>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut sql = String::from(
            "SELECT id, provider_id, model, field, old_value, new_value, source_before, source_after, changed_at
             FROM capability_audit",
        );
        let mut params_dyn: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let mut clauses: Vec<&str> = Vec::new();
        if let Some(p) = provider_id {
            clauses.push("provider_id = ?");
            params_dyn.push(Box::new(p.to_string()));
        }
        if let Some(m) = model {
            clauses.push("model = ?");
            params_dyn.push(Box::new(m.to_string()));
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY changed_at DESC LIMIT ?");
        params_dyn.push(Box::new(limit));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params_dyn.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), capability_audit_from_row)?;
        collect_rows(rows)
    }

    /// T33: drop every capability row for a provider. Next resolve will re-
    /// persist the builtin defaults (and write a capability_audit row for the
    /// new builtin source). Returns rows deleted.
    pub fn reset_capabilities_for_provider(&self, provider_id: &str) -> StorageResult<usize> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let deleted = conn.execute(
            "DELETE FROM provider_capabilities WHERE provider_id = ?1",
            params![provider_id],
        )?;
        Ok(deleted)
    }

    fn all_artifacts(&self) -> StorageResult<Vec<ArtifactRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, run_id, kind, title, path, operation, status, summary, metadata, created_at, updated_at
             FROM artifacts ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], artifact_from_row)?;
        collect_rows(rows)
    }

    pub fn export_local_data(&self) -> StorageResult<LocalDataExport> {
        Ok(LocalDataExport {
            schema_version: 8,
            exported_at: now_ms(),
            db_path: self.path.to_string_lossy().to_string(),
            sessions: self.list_all_sessions()?,
            messages: self.all_messages()?,
            projects: self.list_all_projects()?,
            agent_runs: self.all_agent_runs()?,
            agent_run_steps: self.all_agent_run_steps()?,
            agent_tool_audit_events: self.all_agent_tool_audit_events()?,
            model_usage_events: self.all_model_usage_events()?,
            plan_tasks: self.all_plan_tasks()?,
            plan_change_events: self.all_plan_change_events()?,
            plugin_packages: self.all_plugin_packages()?,
            plugin_capability_events: self.all_plugin_capability_events()?,
            artifacts: self.all_artifacts()?,
            memories: self.list_memories()?,
            profile: self.get_profile()?,
            personality_progress: self.get_personality_progress()?,
            app_state: self.list_app_state_for_export()?,
            activity_events: self.recent_activity_events(None, 5000)?,
            provider_capabilities: self.list_provider_capabilities()?,
            capability_audit: self.list_capability_audit(None, None, 10_000)?,
        })
    }

    pub fn write_export_file(&self) -> StorageResult<PathBuf> {
        let export = self.export_local_data()?;
        let dir = atlas_home()?.join("exports");
        std::fs::create_dir_all(&dir)?;
        let filename = format!("atlas-export-{}.json", Utc::now().format("%Y%m%d-%H%M%S"));
        let path = dir.join(filename);
        std::fs::write(&path, serde_json::to_string_pretty(&export)?)?;
        Ok(path)
    }

    pub fn reset_local_data(
        &self,
        options: ResetLocalDataOptions,
    ) -> StorageResult<ResetLocalDataSummary> {
        let mut scopes = Vec::new();
        let mut conn = self.conn.lock().expect("local db mutex poisoned");
        let tx = conn.transaction()?;

        if options.sessions {
            tx.execute("DELETE FROM agent_run_steps", [])?;
            tx.execute("DELETE FROM agent_tool_audit_events", [])?;
            tx.execute("DELETE FROM permission_decisions", [])?;
            tx.execute("DELETE FROM model_usage_events", [])?;
            tx.execute("DELETE FROM run_file_checkpoints", [])?;
            tx.execute("DELETE FROM run_task_verifications", [])?;
            tx.execute("DELETE FROM plan_change_events", [])?;
            tx.execute("DELETE FROM run_plans", [])?;
            tx.execute("DELETE FROM plan_tasks", [])?;
            tx.execute("DELETE FROM artifacts", [])?;
            tx.execute("DELETE FROM agent_runs", [])?;
            tx.execute("DELETE FROM pending_commands", [])?;
            tx.execute("DELETE FROM pending_file_writes", [])?;
            tx.execute("DELETE FROM messages", [])?;
            tx.execute("DELETE FROM sessions", [])?;
            tx.execute("DELETE FROM projects", [])?;
            tx.execute("DELETE FROM activity_events", [])?;
            scopes.push("sessions".to_string());
        }
        if options.memories {
            tx.execute("DELETE FROM memories", [])?;
            scopes.push("memories".to_string());
        }
        if options.profile {
            tx.execute("DELETE FROM profiles", [])?;
            tx.execute("DELETE FROM personality_progress", [])?;
            scopes.push("profile".to_string());
        }
        if options.app_state {
            tx.execute(
                "DELETE FROM app_state WHERE key NOT LIKE 'provider_field:%'",
                [],
            )?;
            scopes.push("app_state".to_string());
        }

        tx.commit()?;
        drop(conn);

        let replacement_session = if options.sessions {
            Some(self.create_session("新会话")?)
        } else {
            None
        };

        Ok(ResetLocalDataSummary {
            reset_scopes: scopes,
            preserved_config: true,
            replacement_session,
            updated_at: now_ms(),
        })
    }

    pub fn health(&self) -> StorageResult<LocalDbHealth> {
        Ok(LocalDbHealth {
            ok: true,
            db_path: self.path.to_string_lossy().to_string(),
            sessions: self.count_table("sessions")?,
            messages: self.count_table("messages")?,
            memories: self.count_table("memories")?,
            activity_events: self.count_table("activity_events")?,
            app_state: self.count_table("app_state")?,
            checked_at: now_ms(),
        })
    }

    fn ensure_session_exists(&self, session_id: &str) -> StorageResult<()> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let exists: Option<String> = conn
            .query_row(
                "SELECT id FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            Ok(())
        } else {
            Err(StorageError::NotFound(session_id.to_string()))
        }
    }

    fn all_messages(&self) -> StorageResult<Vec<MessageRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, created_at, metadata
             FROM messages ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let metadata: String = row.get(5)?;
            Ok(MessageRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                metadata: serde_json::from_str(&metadata).unwrap_or_else(|_| json!({})),
            })
        })?;
        collect_rows(rows)
    }

    fn list_app_state(&self) -> StorageResult<Vec<AppStateRecord>> {
        let conn = self.conn.lock().expect("local db mutex poisoned");
        let mut stmt =
            conn.prepare("SELECT key, value_json, updated_at FROM app_state ORDER BY key ASC")?;
        let rows = stmt.query_map([], |row| {
            let raw: String = row.get(1)?;
            Ok(AppStateRecord {
                key: row.get(0)?,
                value: serde_json::from_str(&raw).unwrap_or(json!(null)),
                updated_at: row.get(2)?,
            })
        })?;
        collect_rows(rows)
    }

    fn list_app_state_for_export(&self) -> StorageResult<Vec<AppStateRecord>> {
        Ok(self
            .list_app_state()?
            .into_iter()
            .map(|mut record| {
                if is_secret_state_key(&record.key) {
                    record.value = json!("***REDACTED***");
                } else {
                    record.value = redact_secret_json(record.value);
                }
                record
            })
            .collect())
    }

    fn count_table(&self, table: &str) -> StorageResult<i64> {
        let sql = match table {
            "sessions" => "SELECT COUNT(*) FROM sessions",
            "messages" => "SELECT COUNT(*) FROM messages",
            "memories" => "SELECT COUNT(*) FROM memories",
            "activity_events" => "SELECT COUNT(*) FROM activity_events",
            "app_state" => "SELECT COUNT(*) FROM app_state",
            _ => return Err(StorageError::NotFound(format!("unknown table: {table}"))),
        };
        let conn = self.conn.lock().expect("local db mutex poisoned");
        Ok(conn.query_row(sql, [], |row| row.get(0))?)
    }
}

fn is_secret_state_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("provider_field")
        || key.contains("api_key")
        || key.contains("apikey")
        || key.contains("auth_token")
        || key.contains("authorization")
        || key.contains("token")
        || key.contains("secret")
        || key.contains("password")
}

fn redact_secret_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(redact_secret_json).collect())
        }
        serde_json::Value::Object(mut object) => {
            let sensitive_pair = object
                .get("key")
                .and_then(|value| value.as_str())
                .map(is_secret_state_key)
                .unwrap_or(false);
            let keys = object.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                if is_secret_state_key(&key)
                    || (sensitive_pair && key.eq_ignore_ascii_case("value"))
                {
                    object.insert(key, json!("***REDACTED***"));
                } else if let Some(value) = object.remove(&key) {
                    object.insert(key, redact_secret_json(value));
                }
            }
            serde_json::Value::Object(object)
        }
        other => other,
    }
}

pub fn atlas_home() -> StorageResult<PathBuf> {
    if let Ok(path) = std::env::var("ATLAS_HOME") {
        let path = path.trim();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    Ok(dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".atlas"))
}

pub fn default_profile_json() -> serde_json::Value {
    json!({
        "personalityType": "",
        "dimensionScores": {},
        "interests": [],
        "tonePreference": "natural",
        "titlePreference": "owner",
        "replyStyle": "professional",
        "testCompletedAt": null,
        "testVersion": 1
    })
}

fn ensure_table_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for item in columns {
        if item? == column {
            return Ok(());
        }
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )?;
    Ok(())
}

fn title_from_message(content: &str) -> String {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact = compact.trim_matches(|ch: char| {
        matches!(
            ch,
            '。' | '！' | '？' | '!' | '?' | '，' | ',' | ':' | '：' | ';' | '；'
        )
    });
    if compact.is_empty() {
        return "新会话".to_string();
    }
    compact.chars().take(22).collect::<String>()
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn collect_rows<T, F>(rows: rusqlite::MappedRows<'_, F>) -> StorageResult<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn session_summary_key(session_id: &str) -> String {
    format!("session_summary:{session_id}")
}

fn normalize_date(date: &str) -> String {
    let trimmed = date.trim();
    if trimmed.is_empty() {
        Utc::now().format("%Y-%m-%d").to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_activity_kind(kind: &str) -> String {
    match kind.trim() {
        "agent" => "agent".to_string(),
        "mood" => "mood".to_string(),
        "memory" => "memory".to_string(),
        "profile" => "profile".to_string(),
        "system" => "system".to_string(),
        _ => "system".to_string(),
    }
}

fn normalize_plan_change_action(action: &str) -> String {
    match action.trim() {
        "create" | "update" | "update_status" | "update_evidence" | "set_active" | "archive"
        | "repair_integrity" => action.trim().to_string(),
        _ => "update".to_string(),
    }
}

fn normalize_plan_change_subject(subject_type: &str) -> String {
    match subject_type.trim() {
        "run_plan" | "plan_task" => subject_type.trim().to_string(),
        _ => "plan_task".to_string(),
    }
}

fn normalize_plugin_event_action(action: &str) -> String {
    match action.trim() {
        "install" | "enable" | "disable" | "invoke" => action.trim().to_string(),
        _ => "invoke".to_string(),
    }
}

fn normalize_plugin_event_status(status: &str) -> String {
    match status.trim() {
        "ok" | "blocked" | "failed" | "preview" => status.trim().to_string(),
        _ => "failed".to_string(),
    }
}

fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn normalize_agent_permission_mode(mode: &str) -> String {
    match mode.trim() {
        "plan" | "safe" | "suggest" => "plan".to_string(),
        "default" | "workspace" | "auto_edit" => "default".to_string(),
        "full_access" | "full" | "full_auto" => "full_access".to_string(),
        _ => "default".to_string(),
    }
}

fn normalize_tool_policy_name(policy: &str) -> String {
    match policy.trim() {
        "full_access" | "allow_all" => "full_access".to_string(),
        "default" | "no_mutation" => "default".to_string(),
        "plan" | "safe_only" => "plan".to_string(),
        "deny_all" => "deny_all".to_string(),
        _ => "default".to_string(),
    }
}

fn normalize_agent_tool_audit_status(status: &str) -> String {
    match status.trim() {
        "allowed" => "allowed".to_string(),
        "blocked" => "blocked".to_string(),
        "executed" => "executed".to_string(),
        "error" => "error".to_string(),
        _ => "error".to_string(),
    }
}

fn normalize_permission_subject(subject: &str) -> String {
    match subject.trim() {
        "file" | "command" | "git" | "network" | "mcp" | "plugin" => subject.trim().to_string(),
        _ => "other".to_string(),
    }
}

fn normalize_permission_risk(risk: &str) -> String {
    match risk.trim() {
        "safe" | "sensitive" | "destructive" => risk.trim().to_string(),
        _ => "sensitive".to_string(),
    }
}

fn normalize_permission_decision(decision: &str) -> String {
    match decision.trim() {
        "allowed" => "allowed".to_string(),
        "needs_confirm" => "needs_confirm".to_string(),
        "denied" => "denied".to_string(),
        // Fail-safe: an unrecognized decision is treated as denied, never allowed.
        _ => "denied".to_string(),
    }
}

fn normalize_permission_decided_by(value: &str) -> String {
    match value.trim() {
        // P1-7: "user" attributes a decision to an explicit human approve/deny of a
        // needs_confirm action (the confirmation event chain), distinct from the
        // automated gate/policy/skill/hard_rule layers.
        "gate" | "policy" | "skill" | "hard_rule" | "user" => value.trim().to_string(),
        _ => "policy".to_string(),
    }
}

fn normalize_agent_run_status(status: &str) -> String {
    match status.trim() {
        "pending" => "pending".to_string(),
        "running" => "running".to_string(),
        "finished" | "completed" | "success" => "finished".to_string(),
        "failed" | "error" => "failed".to_string(),
        "cancelled" | "canceled" => "cancelled".to_string(),
        "paused" => "paused".to_string(),
        "blocked" => "blocked".to_string(),
        _ => "running".to_string(),
    }
}

fn normalize_agent_step_type(step_type: &str) -> String {
    match step_type.trim() {
        "iteration" => "iteration".to_string(),
        "model_route" => "model_route".to_string(),
        "model_call" => "model_call".to_string(),
        "tool_call" => "tool_call".to_string(),
        "tool_result" => "tool_result".to_string(),
        "file_change" => "file_change".to_string(),
        "command" => "command".to_string(),
        "approval" => "approval".to_string(),
        "response" => "response".to_string(),
        "guidance" => "guidance".to_string(),
        "thinking" => "thinking".to_string(),
        "protocol_task" => "protocol_task".to_string(),
        "workflow_queue" => "workflow_queue".to_string(),
        _ => "event".to_string(),
    }
}

fn normalize_agent_step_status(status: &str) -> String {
    match status.trim() {
        "pending" => "pending".to_string(),
        "running" => "running".to_string(),
        "finished" | "completed" | "success" => "finished".to_string(),
        "failed" | "error" => "failed".to_string(),
        "cancelled" | "canceled" => "cancelled".to_string(),
        _ => "finished".to_string(),
    }
}

fn normalize_browser_action(action: &str) -> String {
    match action.trim() {
        "search" | "open" | "screenshot" | "click" | "type" | "press" => action.trim().to_string(),
        _ => "unknown".to_string(),
    }
}

fn normalize_browser_agent_status(status: &str) -> String {
    match status.trim() {
        "pending" | "ready" | "observed" | "uncertain" | "blocked" | "failed" => {
            status.trim().to_string()
        }
        "ok" | "success" => "observed".to_string(),
        "error" => "failed".to_string(),
        _ => "uncertain".to_string(),
    }
}

fn normalize_graph_run_status(status: &str) -> String {
    match status.trim() {
        "pending" | "running" | "paused" | "succeeded" | "failed" | "blocked" | "cancelled" => {
            status.trim().to_string()
        }
        "finished" | "completed" | "success" => "succeeded".to_string(),
        "error" => "failed".to_string(),
        "canceled" => "cancelled".to_string(),
        _ => "pending".to_string(),
    }
}

fn normalize_graph_node_status(status: &str) -> String {
    match status.trim() {
        "pending" | "running" | "succeeded" | "failed" | "skipped" | "blocked" => {
            status.trim().to_string()
        }
        "finished" | "completed" | "success" => "succeeded".to_string(),
        "error" => "failed".to_string(),
        _ => "pending".to_string(),
    }
}

fn normalize_graph_node_kind(kind: &str) -> String {
    match kind.trim() {
        "agent" | "tool" | "verifier" | "router" | "checkpoint" | "human_gate" => {
            kind.trim().to_string()
        }
        _ => "agent".to_string(),
    }
}

fn normalize_json_object(value: serde_json::Value) -> serde_json::Value {
    if value.is_null() {
        json!({})
    } else {
        value
    }
}

fn mask_log_value(value: serde_json::Value) -> serde_json::Value {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    let masked = crate::tools::secret_scan::scan(
        &serialized,
        crate::tools::secret_scan::SecretLocation::Log,
        crate::tools::secret_scan::SecretAction::Masked,
    )
    .text;
    serde_json::from_str(&masked).unwrap_or(serde_json::Value::String(masked))
}

fn normalize_team_run_status(status: &str) -> String {
    match status.trim() {
        "running" | "paused" | "blocked" | "completed" | "failed" | "cancelled" => {
            status.trim().to_string()
        }
        "done" | "finished" | "success" => "completed".to_string(),
        "error" => "failed".to_string(),
        "canceled" => "cancelled".to_string(),
        _ => "running".to_string(),
    }
}

fn normalize_team_role(role: &str) -> String {
    match role.trim() {
        "main" | "planner" | "executor" | "verifier" | "reviewer" | "tester" | "researcher" => {
            role.trim().to_string()
        }
        _ => "reviewer".to_string(),
    }
}

fn normalize_team_message_role(role: &str) -> String {
    match role.trim() {
        "system" | "main" | "planner" | "executor" | "verifier" | "reviewer" | "tester"
        | "researcher" => role.trim().to_string(),
        _ => "system".to_string(),
    }
}

fn normalize_team_message_type(message_type: &str) -> String {
    match message_type.trim() {
        "task" | "handoff" | "evidence" | "observation" | "proposal" | "main_review" | "status" => {
            message_type.trim().to_string()
        }
        // Subagents are not allowed to persist a direct completion claim.
        "complete" | "completion" | "completion_claim" => "proposal".to_string(),
        _ => "observation".to_string(),
    }
}

fn normalize_handoff_status(status: &str) -> String {
    match status.trim() {
        "pending" | "accepted" | "rejected" | "completed" | "cancelled" => {
            status.trim().to_string()
        }
        "done" | "finished" => "completed".to_string(),
        "canceled" => "cancelled".to_string(),
        _ => "pending".to_string(),
    }
}

fn normalize_knowledge_scope(scope: &str) -> String {
    let scope = compact_text(scope.trim(), 160).to_ascii_lowercase();
    if scope.is_empty() {
        "global".to_string()
    } else {
        scope
    }
}

fn normalize_knowledge_trust(trust: &str) -> String {
    match trust.trim() {
        "trusted" | "user" | "project" | "tool" | "external" | "untrusted" => {
            trust.trim().to_string()
        }
        _ => "external".to_string(),
    }
}

fn knowledge_trust_weight(trust: &str) -> f64 {
    match trust {
        "trusted" | "user" | "project" => 1.0,
        "tool" => 0.85,
        "external" => 0.65,
        "untrusted" => 0.35,
        _ => 0.5,
    }
}

fn retrieval_terms(value: &str) -> BTreeSet<String> {
    value
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
        .map(str::trim)
        .filter(|term| term.chars().count() >= 2)
        .map(|term| term.to_ascii_lowercase())
        .collect()
}

fn retrieval_hit_for_item(
    query_terms: &BTreeSet<String>,
    item: &KnowledgeItemRecord,
) -> Option<RetrievalHitRecord> {
    let haystack = format!("{} {}", item.title, item.text);
    let item_terms = retrieval_terms(&haystack);
    let overlap = query_terms
        .iter()
        .filter(|term| item_terms.contains(*term))
        .count();
    if overlap == 0 {
        return None;
    }
    let coverage = overlap as f64 / query_terms.len().max(1) as f64;
    let score = coverage * item.confidence * knowledge_trust_weight(&item.trust);
    if score <= 0.0 {
        return None;
    }
    Some(RetrievalHitRecord {
        item_id: item.id.clone(),
        scope: item.scope.clone(),
        source: item.source.clone(),
        trust: item.trust.clone(),
        title: item.title.clone(),
        snippet: retrieval_snippet(&item.text, query_terms, 240),
        score,
        confidence: item.confidence,
        reason: format!(
            "lexical_overlap={} coverage={:.2} trust={} source={}",
            overlap, coverage, item.trust, item.source
        ),
        embedding_ref: item.embedding_ref.clone(),
        created_at: item.created_at,
    })
}

fn retrieval_snippet(text: &str, terms: &BTreeSet<String>, max_chars: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = compact.to_ascii_lowercase();
    let start = terms
        .iter()
        .filter_map(|term| lower.find(term))
        .min()
        .unwrap_or(0);
    let prefix = start.saturating_sub(60);
    let snippet = compact
        .chars()
        .skip(prefix)
        .take(max_chars)
        .collect::<String>();
    if prefix > 0 {
        format!("...{snippet}")
    } else {
        snippet
    }
}

fn stable_text_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn normalize_sandbox_backend(value: Option<&str>) -> String {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some("local") | None => "local".to_string(),
        Some("namespace") => "namespace".to_string(),
        Some("container") => "container".to_string(),
        Some("external") => "external".to_string(),
        Some(_) => "local".to_string(),
    }
}

fn normalize_workspace_setup_status(status: &str) -> String {
    match status.trim() {
        "pending" | "running" | "succeeded" | "failed" | "skipped" => status.trim().to_string(),
        "ok" | "success" | "passed" => "succeeded".to_string(),
        "error" => "failed".to_string(),
        _ => "pending".to_string(),
    }
}

fn normalize_workspace_stage(stage: &str) -> String {
    match stage.trim() {
        "created" | "preparing_repo" | "setup" | "git_hooks" | "sandbox" | "ready" | "archive" => {
            stage.trim().to_string()
        }
        _ => "setup".to_string(),
    }
}

fn normalize_workspace_root(root_path: &str) -> StorageResult<String> {
    let raw = root_path.trim();
    if raw.is_empty() {
        return Err(StorageError::Validation(
            "workspace root path is empty".to_string(),
        ));
    }
    let path = PathBuf::from(raw);
    let real = path.canonicalize()?;
    if !real.is_dir() {
        return Err(StorageError::Validation(
            "workspace root is not a directory".to_string(),
        ));
    }
    if crate::tools::execution_isolation::is_sensitive_path(&real) {
        return Err(StorageError::Validation(
            "workspace root is a sensitive path".to_string(),
        ));
    }
    Ok(real.to_string_lossy().to_string())
}

fn normalize_write_target(path: &Path) -> StorageResult<PathBuf> {
    let mut raw = path.to_string_lossy().to_string();
    if raw.starts_with("~/") || raw.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            raw = home
                .join(raw.trim_start_matches("~/").trim_start_matches("~\\"))
                .to_string_lossy()
                .to_string();
        }
    }
    let path = PathBuf::from(raw);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    canonicalize_write_path(&absolute)
}

fn canonicalize_write_path(path: &Path) -> StorageResult<PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }

    let mut missing: Vec<OsString> = Vec::new();
    let mut cursor = path;
    while !cursor.exists() {
        let Some(name) = cursor.file_name() else {
            return Err(StorageError::Validation(
                "无法解析目标路径的已存在父目录。".to_string(),
            ));
        };
        missing.push(name.to_os_string());
        let Some(parent) = cursor.parent() else {
            return Err(StorageError::Validation(
                "无法解析目标路径的已存在父目录。".to_string(),
            ));
        };
        cursor = parent;
    }

    let mut real = cursor.canonicalize()?;
    for segment in missing.iter().rev() {
        real.push(segment);
    }
    Ok(real)
}

fn validate_write_target(path: &Path) -> StorageResult<()> {
    let path_text = path.to_string_lossy().to_ascii_lowercase();
    let blocked = [
        "\\windows\\",
        "\\program files\\",
        "\\program files (x86)\\",
        "\\appdata\\local\\google\\chrome\\",
        "\\appdata\\roaming\\mozilla\\",
        "\\appdata\\roaming\\telegram",
        "\\appdata\\roaming\\discord",
        "\\.ssh\\",
        "\\.gnupg\\",
    ];
    if blocked.iter().any(|item| path_text.contains(item)) {
        return Err(StorageError::Validation(
            "目标路径属于系统、密钥或敏感应用目录，Atlas 不会写入。".to_string(),
        ));
    }
    Ok(())
}

fn activity_event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActivityEvent> {
    let metadata: String = row.get(5)?;
    Ok(ActivityEvent {
        id: row.get(0)?,
        date: row.get(1)?,
        kind: row.get(2)?,
        title: row.get(3)?,
        detail: row.get(4)?,
        metadata: serde_json::from_str(&metadata).unwrap_or_else(|_| json!({})),
        created_at: row.get(6)?,
    })
}

fn agent_tool_audit_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentToolAuditRecord> {
    Ok(AgentToolAuditRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        run_id: row.get(2)?,
        iteration: row.get(3)?,
        tool_call_id: row.get(4)?,
        tool_name: row.get(5)?,
        permission_mode: row.get(6)?,
        policy: row.get(7)?,
        status: row.get(8)?,
        reason: row.get(9)?,
        created_at: row.get(10)?,
    })
}

fn permission_decision_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PermissionDecisionRecord> {
    Ok(PermissionDecisionRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        run_id: row.get(2)?,
        iteration: row.get(3)?,
        tool_call_id: row.get(4)?,
        subject: row.get(5)?,
        action: row.get(6)?,
        risk: row.get(7)?,
        mode: row.get(8)?,
        decision: row.get(9)?,
        reason: row.get(10)?,
        decided_by: row.get(11)?,
        created_at: row.get(12)?,
    })
}

fn model_usage_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelUsageRecord> {
    Ok(ModelUsageRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        run_id: row.get(2)?,
        iteration: row.get(3)?,
        provider: row.get(4)?,
        model: row.get(5)?,
        input_tokens: row.get(6)?,
        output_tokens: row.get(7)?,
        total_tokens: row.get(8)?,
        source: row.get(9)?,
        created_at: row.get(10)?,
    })
}

fn pending_command_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingCommandRecord> {
    Ok(PendingCommandRecord {
        id: row.get(0)?,
        command: row.get(1)?,
        cwd: row.get(2)?,
        reason: row.get(3)?,
        shell: row.get(4)?,
        created_at: row.get(5)?,
    })
}

/// P1-3: sort merged timeline entries chronologically with a deterministic
/// tiebreak, then return the requested page plus the full `total`. Kept pure
/// (no IO) so ordering and pagination are unit-testable with controlled
/// timestamps. `limit`/`offset` are caller-clamped row counts.
fn paginate_run_timeline(
    mut entries: Vec<RunTimelineEntry>,
    limit: usize,
    offset: usize,
) -> (i64, Vec<RunTimelineEntry>) {
    entries.sort_by(|a, b| {
        a.at.cmp(&b.at)
            .then(a.seq.cmp(&b.seq))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.id.cmp(&b.id))
    });
    let total = entries.len() as i64;
    let start = offset.min(entries.len());
    let end = start.saturating_add(limit).min(entries.len());
    (total, entries[start..end].to_vec())
}

fn agent_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRunRecord> {
    Ok(AgentRunRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        status: row.get(2)?,
        permission_mode: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        finished_at: row.get(6)?,
        error: row.get(7)?,
    })
}

fn agent_run_step_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRunStepRecord> {
    let input: String = row.get(6)?;
    let output: String = row.get(7)?;
    Ok(AgentRunStepRecord {
        id: row.get(0)?,
        run_id: row.get(1)?,
        step_index: row.get(2)?,
        step_type: row.get(3)?,
        status: row.get(4)?,
        summary: row.get(5)?,
        input: serde_json::from_str(&input).unwrap_or_else(|_| json!({})),
        output: serde_json::from_str(&output).unwrap_or_else(|_| json!({})),
        created_at: row.get(8)?,
        finished_at: row.get(9)?,
    })
}

fn browser_agent_step_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<BrowserAgentStepRecord> {
    let dom_summary: String = row.get(10)?;
    let action_json: String = row.get(11)?;
    let result_json: String = row.get(12)?;
    let judge_json: String = row.get(14)?;
    Ok(BrowserAgentStepRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        run_id: row.get(2)?,
        step_index: row.get(3)?,
        action: row.get(4)?,
        target: row.get(5)?,
        status: row.get(6)?,
        title: row.get(7)?,
        url: row.get(8)?,
        screenshot_path: row.get(9)?,
        dom_summary: serde_json::from_str(&dom_summary).unwrap_or_else(|_| json!({})),
        action_json: serde_json::from_str(&action_json).unwrap_or_else(|_| json!({})),
        result_json: serde_json::from_str(&result_json).unwrap_or_else(|_| json!({})),
        fingerprint: row.get(13)?,
        judge: serde_json::from_str(&judge_json).unwrap_or_else(|_| json!({})),
        loop_detected: row.get::<_, i64>(15)? != 0,
        created_at: row.get(16)?,
    })
}

fn agent_graph_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentGraphRunRecord> {
    Ok(AgentGraphRunRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        source_run_id: row.get(2)?,
        goal: row.get(3)?,
        status: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        finished_at: row.get(7)?,
        error: row.get(8)?,
    })
}

fn agent_graph_node_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentGraphNodeRecord> {
    let input_json: String = row.get(8)?;
    let output_json: String = row.get(9)?;
    Ok(AgentGraphNodeRecord {
        id: row.get(0)?,
        graph_run_id: row.get(1)?,
        node_key: row.get(2)?,
        kind: row.get(3)?,
        title: row.get(4)?,
        status: row.get(5)?,
        attempt: row.get(6)?,
        max_attempts: row.get(7)?,
        input: serde_json::from_str(&input_json).unwrap_or_else(|_| json!({})),
        output: serde_json::from_str(&output_json).unwrap_or_else(|_| json!({})),
        error: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        started_at: row.get(13)?,
        finished_at: row.get(14)?,
    })
}

fn agent_graph_edge_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentGraphEdgeRecord> {
    Ok(AgentGraphEdgeRecord {
        id: row.get(0)?,
        graph_run_id: row.get(1)?,
        from_node_id: row.get(2)?,
        to_node_id: row.get(3)?,
        condition: row.get(4)?,
        created_at: row.get(5)?,
    })
}

fn agent_graph_checkpoint_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<AgentGraphCheckpointRecord> {
    let state_json: String = row.get(3)?;
    Ok(AgentGraphCheckpointRecord {
        id: row.get(0)?,
        graph_run_id: row.get(1)?,
        node_id: row.get(2)?,
        state: serde_json::from_str(&state_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(4)?,
    })
}

fn team_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TeamRunRecord> {
    Ok(TeamRunRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        source_run_id: row.get(2)?,
        goal: row.get(3)?,
        status: row.get(4)?,
        max_rounds: row.get(5)?,
        termination_reason: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        finished_at: row.get(9)?,
    })
}

fn team_participant_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TeamParticipantRecord> {
    let tool_scope_json: String = row.get(5)?;
    Ok(TeamParticipantRecord {
        id: row.get(0)?,
        team_run_id: row.get(1)?,
        name: row.get(2)?,
        role: row.get(3)?,
        model: row.get(4)?,
        tool_scope: serde_json::from_str(&tool_scope_json).unwrap_or_else(|_| json!({})),
        status: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn team_message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TeamMessageRecord> {
    let metadata_json: String = row.get(6)?;
    Ok(TeamMessageRecord {
        id: row.get(0)?,
        team_run_id: row.get(1)?,
        participant_id: row.get(2)?,
        role: row.get(3)?,
        message_type: row.get(4)?,
        content: row.get(5)?,
        metadata: serde_json::from_str(&metadata_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(7)?,
    })
}

fn handoff_request_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HandoffRequestRecord> {
    let contract_json: String = row.get(6)?;
    let result_json: String = row.get(7)?;
    Ok(HandoffRequestRecord {
        id: row.get(0)?,
        team_run_id: row.get(1)?,
        from_participant_id: row.get(2)?,
        to_participant_id: row.get(3)?,
        status: row.get(4)?,
        reason: row.get(5)?,
        contract: serde_json::from_str(&contract_json).unwrap_or_else(|_| json!({})),
        result: serde_json::from_str(&result_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(8)?,
        resolved_at: row.get(9)?,
    })
}

fn knowledge_item_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeItemRecord> {
    Ok(KnowledgeItemRecord {
        id: row.get(0)?,
        scope: row.get(1)?,
        source: row.get(2)?,
        trust: row.get(3)?,
        title: row.get(4)?,
        text: row.get(5)?,
        enabled: row.get::<_, i64>(6)? != 0,
        confidence: row.get(7)?,
        expires_at: row.get(8)?,
        embedding_ref: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        deleted_at: row.get(12)?,
    })
}

fn workspace_lifecycle_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WorkspaceLifecycleRecord> {
    let audit_json: String = row.get(10)?;
    Ok(WorkspaceLifecycleRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        run_id: row.get(2)?,
        root_path: row.get(3)?,
        status: row.get(4)?,
        setup_status: row.get(5)?,
        sandbox_backend: row.get(6)?,
        sandbox_status: row.get(7)?,
        fallback_reason: row.get(8)?,
        setup_script: row.get(9)?,
        audit: serde_json::from_str(&audit_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        archived_at: row.get(13)?,
    })
}

fn workspace_setup_event_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WorkspaceSetupEventRecord> {
    Ok(WorkspaceSetupEventRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        stage: row.get(2)?,
        status: row.get(3)?,
        command: row.get(4)?,
        exit_code: row.get(5)?,
        output_tail: row.get(6)?,
        reason: row.get(7)?,
        created_at: row.get(8)?,
    })
}

fn eval_run_storage_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EvalRunStorageRecord> {
    let report_json: String = row.get(9)?;
    Ok(EvalRunStorageRecord {
        id: row.get(0)?,
        suite_id: row.get(1)?,
        provider: row.get(2)?,
        model: row.get(3)?,
        status: row.get(4)?,
        cwd: row.get(5)?,
        passed: row.get::<_, i64>(6)? != 0,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
        report: serde_json::from_str(&report_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(10)?,
    })
}

fn plan_task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanTaskRecord> {
    let acceptance_json: Option<String> = row.get(11)?;
    let verify_json: Option<String> = row.get(12)?;
    let evidence_json: Option<String> = row.get(13)?;
    let evidence_status: Option<String> = row.get(14)?;
    let active: Option<i64> = row.get(15)?;
    let blocked_reason: Option<String> = row.get(16)?;
    Ok(PlanTaskRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        run_id: row.get(2)?,
        parent_id: row.get(3)?,
        title: row.get(4)?,
        status: row.get(5)?,
        position: row.get(6)?,
        source: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        archived_at: row.get(10)?,
        acceptance_criteria: acceptance_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(json!(null)),
        verify: verify_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(json!(null)),
        evidence: evidence_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(json!(null)),
        evidence_status: evidence_status.unwrap_or_else(default_evidence_status),
        active: active.unwrap_or(0) != 0,
        blocked_reason,
    })
}

fn run_plan_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunPlanRecord> {
    let observable_outcome: Option<String> = row.get(4)?;
    let non_goals_json: Option<String> = row.get(5)?;
    let acceptance_json: Option<String> = row.get(6)?;
    Ok(RunPlanRecord {
        id: row.get(0)?,
        run_id: row.get(1)?,
        session_id: row.get(2)?,
        goal: row.get(3)?,
        observable_outcome: observable_outcome.filter(|s| !s.is_empty()),
        non_goals: non_goals_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_else(|| json!([])),
        acceptance_criteria: acceptance_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(json!(null)),
        status: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn plan_change_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanChangeRecord> {
    let before_json: String = row.get(8)?;
    let after_json: String = row.get(9)?;
    Ok(PlanChangeRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        run_id: row.get(2)?,
        actor: row.get(3)?,
        action: row.get(4)?,
        subject_type: row.get(5)?,
        subject_id: row.get(6)?,
        reason: row.get(7)?,
        before: serde_json::from_str(&before_json).unwrap_or(Value::Null),
        after: serde_json::from_str(&after_json).unwrap_or(Value::Null),
        created_at: row.get(10)?,
    })
}

fn plugin_package_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PluginPackageRecord> {
    let permissions_json: String = row.get(8)?;
    let capabilities_json: String = row.get(9)?;
    let manifest_json: String = row.get(10)?;
    let trusted: i64 = row.get(5)?;
    let enabled: i64 = row.get(6)?;
    Ok(PluginPackageRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        version: row.get(2)?,
        source: row.get(3)?,
        description: row.get(4)?,
        trusted: trusted != 0,
        enabled: enabled != 0,
        risk: row.get(7)?,
        permissions: serde_json::from_str(&permissions_json).unwrap_or_else(|_| json!([])),
        capabilities: serde_json::from_str(&capabilities_json).unwrap_or_else(|_| json!([])),
        manifest: serde_json::from_str(&manifest_json).unwrap_or_else(|_| json!({})),
        installed_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn plugin_capability_event_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PluginCapabilityEventRecord> {
    let input_json: String = row.get(7)?;
    let output_json: String = row.get(8)?;
    Ok(PluginCapabilityEventRecord {
        id: row.get(0)?,
        plugin_id: row.get(1)?,
        capability_id: row.get(2)?,
        action: row.get(3)?,
        status: row.get(4)?,
        risk: row.get(5)?,
        reason: row.get(6)?,
        input: serde_json::from_str(&input_json).unwrap_or(Value::Null),
        output: serde_json::from_str(&output_json).unwrap_or(Value::Null),
        created_at: row.get(9)?,
    })
}

fn task_verification_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskVerificationRecord> {
    Ok(TaskVerificationRecord {
        id: row.get(0)?,
        run_id: row.get(1)?,
        task_id: row.get(2)?,
        kind: row.get(3)?,
        command: row.get(4)?,
        exit_code: row.get(5)?,
        status: row.get(6)?,
        stdout_tail: row.get(7)?,
        stderr_tail: row.get(8)?,
        started_at: row.get(9)?,
        finished_at: row.get(10)?,
    })
}

fn file_checkpoint_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileCheckpointRecord> {
    Ok(FileCheckpointRecord {
        id: row.get(0)?,
        run_id: row.get(1)?,
        task_id: row.get(2)?,
        path: row.get(3)?,
        before_hash: row.get(4)?,
        after_hash: row.get(5)?,
        after_content: row.get(6)?,
        before_content: row.get(7)?,
        before_blob_path: row.get(8)?,
        before_size: row.get(9)?,
        created_at: row.get(10)?,
        restored_at: row.get(11)?,
    })
}

fn provider_capabilities_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProviderCapabilitiesRow> {
    Ok(ProviderCapabilitiesRow {
        provider_id: row.get(0)?,
        model: row.get(1)?,
        vision: row.get::<_, i64>(2)? != 0,
        tool_calls: row.get::<_, i64>(3)? != 0,
        json_mode: row.get::<_, i64>(4)? != 0,
        max_context: row.get::<_, i64>(5)? as u32,
        source: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn capability_audit_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CapabilityAuditRow> {
    Ok(CapabilityAuditRow {
        id: row.get(0)?,
        provider_id: row.get(1)?,
        model: row.get(2)?,
        field: row.get(3)?,
        old_value: row.get(4)?,
        new_value: row.get(5)?,
        source_before: row.get(6)?,
        source_after: row.get(7)?,
        changed_at: row.get(8)?,
    })
}

fn artifact_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactRecord> {
    let metadata: String = row.get(9)?;
    Ok(ArtifactRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        run_id: row.get(2)?,
        kind: row.get(3)?,
        title: row.get(4)?,
        path: row.get(5)?,
        operation: row.get(6)?,
        status: row.get(7)?,
        summary: row.get(8)?,
        metadata: serde_json::from_str(&metadata).unwrap_or_else(|_| json!({})),
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn compact_text(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        compact
    } else {
        let mut out = compact.chars().take(max_chars).collect::<String>();
        out.push_str("...");
        out
    }
}

fn clean_optional_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| compact_text(value, 96))
}

fn normalize_plan_task_status(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "pending" | "running" | "waiting" | "failed" | "done" | "cancelled" | "skipped"
        | "blocked" | "doing" | "verifying" | "waived" | "todo" => {
            value.trim().to_ascii_lowercase()
        }
        "finished" | "complete" | "completed" => "done".to_string(),
        _ => "pending".to_string(),
    }
}

fn normalize_evidence_status(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" | "pending" | "verified" | "waived" | "failed" => value.trim().to_ascii_lowercase(),
        _ => "none".to_string(),
    }
}

fn normalize_verification_kind(value: &str) -> String {
    let kind = compact_text(value.trim(), 120).to_ascii_lowercase();
    if kind.is_empty() {
        "manual".to_string()
    } else {
        kind
    }
}

fn normalize_verification_status(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "passed" | "failed" | "skipped" | "waived" | "running" => value.trim().to_ascii_lowercase(),
        "ok" | "success" => "passed".to_string(),
        _ => "running".to_string(),
    }
}

fn tail_text(value: &str, max_chars: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max_chars {
        value.to_string()
    } else {
        chars[chars.len() - max_chars..].iter().collect()
    }
}

fn normalize_artifact_kind(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "file" | "diff" | "html" | "image" | "report" | "command" => {
            value.trim().to_ascii_lowercase()
        }
        _ => "file".to_string(),
    }
}

fn normalize_artifact_status(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "pending" | "approved" | "rejected" | "written" | "failed" => {
            value.trim().to_ascii_lowercase()
        }
        "finished" | "done" => "written".to_string(),
        _ => "pending".to_string(),
    }
}

fn normalize_memory_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn file_write_diff(existing: Option<&str>, next: &str) -> String {
    let Some(existing) = existing else {
        let preview = next
            .lines()
            .take(80)
            .map(|line| format!("+{line}"))
            .collect::<Vec<_>>()
            .join("\n");
        return if preview.is_empty() {
            "+".to_string()
        } else {
            preview
        };
    };
    if existing == next {
        return "内容没有变化。".to_string();
    }
    let old_lines: Vec<&str> = existing.lines().collect();
    let new_lines: Vec<&str> = next.lines().collect();
    let mut out = Vec::new();
    out.push("--- 当前文件".to_string());
    out.push("+++ 新内容".to_string());
    let max = old_lines.len().max(new_lines.len()).min(120);
    for index in 0..max {
        let old = old_lines.get(index).copied();
        let new = new_lines.get(index).copied();
        if old == new {
            if out.len() < 24 {
                out.push(format!(" {}", old.unwrap_or("")));
            }
            continue;
        }
        if let Some(old) = old {
            out.push(format!("-{old}"));
        }
        if let Some(new) = new {
            out.push(format!("+{new}"));
        }
        if out.len() > 160 {
            out.push("... diff 已截断".to_string());
            break;
        }
    }
    out.join("\n")
}

fn parse_history_report_range(range: Option<&str>, now: i64) -> (String, Option<i64>) {
    let raw = range.unwrap_or("").trim().to_ascii_lowercase();
    if raw.is_empty()
        || raw == "7d"
        || raw.contains("7")
        || raw.contains("week")
        || raw.contains("周")
    {
        return ("最近 7 天".to_string(), Some(now - 7 * 24 * 60 * 60 * 1000));
    }
    if raw == "today" || raw.contains("今天") || raw.contains("今日") {
        return ("最近 24 小时".to_string(), Some(now - 24 * 60 * 60 * 1000));
    }
    if raw == "30d" || raw.contains("30") || raw.contains("month") || raw.contains("月") {
        return (
            "最近 30 天".to_string(),
            Some(now - 30 * 24 * 60 * 60 * 1000),
        );
    }
    if raw == "all" || raw.contains("全部") || raw.contains("所有") {
        return ("全部历史".to_string(), None);
    }
    ("最近 7 天".to_string(), Some(now - 7 * 24 * 60 * 60 * 1000))
}

fn looks_like_question(value: &str) -> bool {
    let compact = value.trim();
    compact.contains('?')
        || compact.contains('？')
        || compact.contains("怎么")
        || compact.contains("为什么")
        || compact.contains("如何")
        || compact.contains("还差")
        || compact.contains("需要")
}

fn export_profile_markdown(profile: &serde_json::Value) -> StorageResult<()> {
    let path = atlas_home()?.join("profile.md");
    let content = format!(
        "# Atlas Profile\n\n```json\n{}\n```\n",
        serde_json::to_string_pretty(profile)?
    );
    std::fs::write(path, content)?;
    Ok(())
}

fn score_personality(answers: &serde_json::Value) -> serde_json::Value {
    let mut sums = std::collections::HashMap::<String, (i64, i64)>::new();
    if let Some(items) = answers.as_array() {
        for item in items {
            let dimension = item
                .get("dimension")
                .and_then(|value| value.as_str())
                .or_else(|| {
                    item.get("questionId")
                        .and_then(|value| value.as_str())
                        .and_then(infer_dimension_from_id)
                })
                .unwrap_or("general")
                .to_string();
            let value = item
                .get("value")
                .and_then(|value| value.as_i64())
                .unwrap_or(3);
            let entry = sums.entry(dimension).or_insert((0, 0));
            entry.0 += value;
            entry.1 += 1;
        }
    }
    let mut output = serde_json::Map::new();
    for (dimension, (sum, count)) in sums {
        output.insert(
            dimension,
            json!((sum as f64 / count.max(1) as f64).round() as i64),
        );
    }
    serde_json::Value::Object(output)
}

fn score_value(scores: &serde_json::Value, key: &str) -> i64 {
    scores.get(key).and_then(|v| v.as_i64()).unwrap_or(3)
}

#[allow(clippy::too_many_arguments)]
fn axis_atom(
    scores: &serde_json::Value,
    key: &str,
    label: &str,
    low_code: &str,
    high_code: &str,
    low_label: &str,
    high_label: &str,
    low_desc: &str,
    high_desc: &str,
) -> (String, serde_json::Value) {
    let score = score_value(scores, key);
    let high = score >= 4;
    let balanced = score == 3;
    let code = if high { high_code } else { low_code };
    let side = if balanced {
        "平衡"
    } else if high {
        high_label
    } else {
        low_label
    };
    let description = if balanced {
        "偏好较平衡，Atlas 应根据上下文动态调整。"
    } else if high {
        high_desc
    } else {
        low_desc
    };
    (
        code.to_string(),
        json!({
            "label": label,
            "side": side,
            "score": score,
            "description": description,
        }),
    )
}

fn build_ati_profile(scores: &serde_json::Value) -> serde_json::Value {
    let axes = [
        axis_atom(
            scores,
            "energy",
            "能量方式",
            "N",
            "C",
            "独处沉浸",
            "协作交互",
            "Atlas 应减少不必要打扰，优先给清楚、低噪声的支持。",
            "Atlas 可以更多使用对话、交换和轻量提醒来推进想法。",
        ),
        axis_atom(
            scores,
            "perception",
            "信息感知",
            "D",
            "V",
            "事实细节",
            "愿景可能",
            "Atlas 应优先给事实、路径、数据和可验证依据。",
            "Atlas 可以先给结构、趋势、可能性和概念地图。",
        ),
        axis_atom(
            scores,
            "decision",
            "决策偏好",
            "L",
            "E",
            "逻辑效率",
            "共情价值",
            "Atlas 应突出利弊、风险、成本和执行效率。",
            "Atlas 应同时照顾感受、沟通方式和关系影响。",
        ),
        axis_atom(
            scores,
            "execution",
            "执行方式",
            "S",
            "F",
            "秩序计划",
            "灵活流动",
            "Atlas 应给明确计划、检查点和收尾动作。",
            "Atlas 应保留弹性，把计划作为可以调整的草稿。",
        ),
    ];

    let mut code = String::new();
    let mut axes_json = serde_json::Map::new();
    for (index, (axis_code, axis_value)) in axes.into_iter().enumerate() {
        code.push_str(&axis_code);
        let key = match index {
            0 => "energy",
            1 => "perception",
            2 => "decision",
            _ => "execution",
        };
        axes_json.insert(key.to_string(), axis_value);
    }

    let energy = axes_json
        .get("energy")
        .and_then(|axis| axis.get("side"))
        .and_then(|value| value.as_str())
        .unwrap_or("平衡");
    let perception = axes_json
        .get("perception")
        .and_then(|axis| axis.get("side"))
        .and_then(|value| value.as_str())
        .unwrap_or("平衡");
    let decision = axes_json
        .get("decision")
        .and_then(|axis| axis.get("side"))
        .and_then(|value| value.as_str())
        .unwrap_or("平衡");
    let execution = axes_json
        .get("execution")
        .and_then(|axis| axis.get("side"))
        .and_then(|value| value.as_str())
        .unwrap_or("平衡");

    json!({
        "code": format!("ATI-{code}"),
        "summary": format!(
            "当前画像偏向：{energy}、{perception}、{decision}、{execution}。这只用于调整 Atlas 的回应方式，不是心理诊断。"
        ),
        "axes": serde_json::Value::Object(axes_json),
    })
}

fn infer_dimension_from_id(id: &str) -> Option<&'static str> {
    let index = id
        .rsplit('_')
        .next()?
        .parse::<usize>()
        .ok()?
        .saturating_sub(1)
        % 12;
    Some(match index {
        0 => "energy",
        1 => "perception",
        2 => "decision",
        3 => "execution",
        4 => "supportMode",
        5 => "proactivity",
        6 => "verbosity",
        7 => "boundary",
        8 => "precision",
        9 => "focus",
        10 => "creativity",
        _ => "privacy",
    })
}

fn infer_interests(answers: &serde_json::Value) -> Vec<String> {
    let scores = score_personality(answers);
    let mut interests = vec!["本地 AI 伙伴".to_string()];
    if scores
        .get("creativity")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        >= 4
    {
        interests.push("创意表达".to_string());
    }
    if scores.get("focus").and_then(|v| v.as_i64()).unwrap_or(0) >= 4 {
        interests.push("信息压缩".to_string());
    }
    interests
}

fn is_legacy_virtual_project(project: &ProjectRecord) -> bool {
    let title = project.title.trim();
    let kind = project.kind.trim().to_ascii_lowercase();
    project
        .root_path
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
        && (title == "全部对话"
            || matches!(
                kind.as_str(),
                "all" | "all_conversations" | "all-conversations" | "virtual"
            ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> LocalDb {
        let path = std::env::temp_dir().join(format!("atlas_test_{}.db", Uuid::new_v4()));
        LocalDb::open(path).unwrap()
    }

    #[test]
    fn initializes_database() {
        let db = temp_db();
        assert!(db.path().exists());
        assert!(db.list_sessions().unwrap().is_empty());
        let conn = db.conn.lock().expect("local db mutex poisoned");
        let current_schema: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(current_schema, 11);
    }

    #[test]
    fn eval_run_report_persists_summary_case_and_commands() {
        let db = temp_db();
        db.persist_eval_run_report(
            PersistEvalRunPayload {
                id: "eval-store".to_string(),
                suite_id: "benchmark".to_string(),
                provider: Some("openai".to_string()),
                model: Some("gpt-4o-mini".to_string()),
                status: "passed".to_string(),
                cwd: ".".to_string(),
                passed: true,
                started_at: 1,
                finished_at: 2,
                report: json!({ "id": "eval-store", "score": { "passed": true } }),
            },
            vec![PersistEvalCaseResultPayload {
                eval_run_id: "eval-store".to_string(),
                case_id: "case-a".to_string(),
                status: "passed".to_string(),
                passed: true,
                verified: true,
                false_completion: false,
                blocked: false,
                artifact_path: Some("target/eval/case-a.json".to_string()),
                result: json!({ "caseId": "case-a" }),
            }],
            vec![PersistEvalCommandResultPayload {
                eval_run_id: "eval-store".to_string(),
                case_id: "case-a".to_string(),
                command: "true".to_string(),
                cwd: ".".to_string(),
                required: true,
                status: "passed".to_string(),
                exit_code: Some(0),
                stdout_tail: String::new(),
                stderr_tail: String::new(),
                started_at: 1,
                finished_at: 2,
                duration_ms: 1,
                timed_out: false,
            }],
        )
        .unwrap();
        let stored = db.get_eval_run_record("eval-store").unwrap().unwrap();
        assert_eq!(stored.suite_id, "benchmark");
        assert!(stored.passed);
        assert_eq!(stored.report["id"], "eval-store");
    }

    #[test]
    fn browser_agent_steps_round_trip_and_join_run_timeline() {
        let db = temp_db();
        db.create_agent_run("run-browser", None, "default").unwrap();
        let step = db
            .record_browser_agent_step(RecordBrowserAgentStepPayload {
                session_id: None,
                run_id: Some("run-browser".to_string()),
                action: "open".to_string(),
                target: None,
                status: "observed".to_string(),
                title: Some("Example".to_string()),
                url: Some("https://example.test".to_string()),
                screenshot_path: None,
                dom_summary: json!({ "totalElements": 10, "interactiveElements": 1 }),
                action_json: json!({ "action": "open" }),
                result_json: json!({ "ok": true }),
                fingerprint: "fp-browser".to_string(),
                judge: json!({ "status": "observed" }),
                loop_detected: false,
            })
            .unwrap();
        assert_eq!(step.step_index, 1);

        let rows = db
            .list_browser_agent_steps(Some("run-browser"), None, 10)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].fingerprint, "fp-browser");
        let timeline = db.get_run_timeline("run-browser", 50, 0).unwrap();
        assert!(timeline
            .entries
            .iter()
            .any(|entry| entry.kind == "browser" && entry.id == step.id));
    }

    #[test]
    fn agent_graph_snapshot_round_trips_nodes_edges_and_checkpoints() {
        let db = temp_db();
        let run = db
            .create_agent_graph_run(CreateAgentGraphRunPayload {
                id: Some("graph-store".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "storage graph".to_string(),
            })
            .unwrap();
        let a = db
            .create_agent_graph_node(CreateAgentGraphNodePayload {
                id: Some("node-a".to_string()),
                graph_run_id: run.id.clone(),
                node_key: "a".to_string(),
                kind: "agent".to_string(),
                title: "Agent".to_string(),
                max_attempts: Some(1),
                input: json!({}),
            })
            .unwrap();
        let b = db
            .create_agent_graph_node(CreateAgentGraphNodePayload {
                id: Some("node-b".to_string()),
                graph_run_id: run.id.clone(),
                node_key: "b".to_string(),
                kind: "tool".to_string(),
                title: "Tool".to_string(),
                max_attempts: Some(2),
                input: json!({ "x": 1 }),
            })
            .unwrap();
        db.create_agent_graph_edge(CreateAgentGraphEdgePayload {
            id: Some("edge-ab".to_string()),
            graph_run_id: run.id.clone(),
            from_node_id: a.id.clone(),
            to_node_id: b.id.clone(),
            condition: Some("success".to_string()),
        })
        .unwrap();
        db.record_agent_graph_checkpoint(&run.id, Some(&a.id), json!({ "ok": true }))
            .unwrap();

        let snapshot = db.get_agent_graph_snapshot(&run.id).unwrap();
        assert_eq!(snapshot.run.goal, "storage graph");
        assert_eq!(snapshot.nodes.len(), 2);
        assert_eq!(snapshot.edges.len(), 1);
        assert_eq!(snapshot.checkpoints.len(), 1);
    }

    fn permission_payload(
        run_id: &str,
        iteration: usize,
        decision: &str,
        decided_by: &str,
    ) -> LogPermissionDecisionPayload {
        LogPermissionDecisionPayload {
            session_id: Some("s1".to_string()),
            run_id: run_id.to_string(),
            iteration,
            tool_call_id: format!("tc-{iteration}"),
            subject: "command".to_string(),
            action: "run_command".to_string(),
            risk: "destructive".to_string(),
            mode: "default".to_string(),
            decision: decision.to_string(),
            reason: "policy_blocks_tool_execution".to_string(),
            decided_by: decided_by.to_string(),
        }
    }

    #[test]
    fn permission_decision_round_trips_and_queries_by_run() {
        let db = temp_db();
        db.log_permission_decision(permission_payload("run-1", 0, "denied", "policy"))
            .unwrap();
        db.log_permission_decision(permission_payload("run-1", 1, "allowed", "gate"))
            .unwrap();
        db.log_permission_decision(permission_payload("run-2", 0, "allowed", "policy"))
            .unwrap();

        let rows = db.permission_decisions_for_run("run-1", 50).unwrap();
        assert_eq!(rows.len(), 2);
        // Oldest first — the run's decision timeline.
        assert_eq!(rows[0].decision, "denied");
        assert_eq!(rows[0].decided_by, "policy");
        assert_eq!(rows[1].decision, "allowed");
        assert_eq!(rows[1].decided_by, "gate");
        // Run isolation.
        assert_eq!(
            db.permission_decisions_for_run("run-2", 50).unwrap().len(),
            1
        );
        assert!(db
            .permission_decisions_for_run("missing", 50)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn permission_decision_normalizes_unknown_values_failsafe() {
        let db = temp_db();
        let record = db
            .log_permission_decision(LogPermissionDecisionPayload {
                session_id: None,
                run_id: "r".to_string(),
                iteration: 0,
                tool_call_id: "t".to_string(),
                subject: "weird".to_string(),
                action: "mystery_tool".to_string(),
                risk: "nonsense".to_string(),
                mode: "bananas".to_string(),
                decision: "maybe".to_string(),
                reason: "x".to_string(),
                decided_by: "ghost".to_string(),
            })
            .unwrap();
        assert_eq!(record.subject, "other");
        assert_eq!(record.risk, "sensitive");
        // Fail-safe: an unrecognized decision must never normalize to allowed.
        assert_eq!(record.decision, "denied");
        assert_eq!(record.decided_by, "policy");
        assert_eq!(record.mode, "default");
    }

    #[test]
    fn permission_decision_rejects_empty_run_id() {
        let db = temp_db();
        let result = db.log_permission_decision(permission_payload("   ", 0, "allowed", "policy"));
        assert!(result.is_err());
    }

    #[test]
    fn permission_decision_confirmation_chain_records_user_resolution() {
        // P1-7: a needs_confirm request followed by an explicit user approval forms
        // the confirmation event chain — both land in the run's decision timeline,
        // and the resolution is attributed to "user", not an automated layer.
        let db = temp_db();
        // gate emits a needs_confirm for the dangerous action...
        db.log_permission_decision(permission_payload("run-c", 0, "needs_confirm", "gate"))
            .unwrap();
        // ...then the user approves it (the P1-7 write-back).
        db.log_permission_decision(LogPermissionDecisionPayload {
            session_id: Some("s1".to_string()),
            run_id: "run-c".to_string(),
            iteration: 0,
            tool_call_id: "tc-0".to_string(),
            subject: "command".to_string(),
            action: "run_command".to_string(),
            risk: "destructive".to_string(),
            mode: "default".to_string(),
            decision: "allowed".to_string(),
            reason: "用户批准危险动作（影响：删除构建产物）".to_string(),
            decided_by: "user".to_string(),
        })
        .unwrap();

        let rows = db.permission_decisions_for_run("run-c", 50).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].decision, "needs_confirm");
        assert_eq!(rows[0].decided_by, "gate");
        // The user resolution is preserved with "user" attribution (not normalized away).
        assert_eq!(rows[1].decision, "allowed");
        assert_eq!(rows[1].decided_by, "user");
        assert!(rows[1].reason.contains("影响"));
    }

    #[test]
    fn reconcile_cancels_running_runs_and_adds_timeline_marker() {
        // P1-1: startup reconcile turns stale `running` runs into cancelled AND
        // leaves a visible "interrupted" marker step on the timeline.
        let db = temp_db();
        db.create_agent_run("run-x", None, "default").unwrap();
        assert_eq!(
            db.get_agent_run("run-x").unwrap().unwrap().status,
            "running"
        );

        let cancelled = db.mark_interrupted_agent_runs().unwrap();
        assert_eq!(cancelled, 1);

        let run = db.get_agent_run("run-x").unwrap().unwrap();
        assert_eq!(run.status, "cancelled");
        assert!(run.error.unwrap_or_default().contains("中断"));

        let steps = db.get_agent_run_steps("run-x").unwrap();
        assert!(
            steps
                .iter()
                .any(|step| step.step_type == "event" && step.summary.contains("异常中断")),
            "expected a reconcile marker step on the timeline"
        );

        // Idempotent: a second reconcile finds nothing running and adds no marker.
        let again = db.mark_interrupted_agent_runs().unwrap();
        assert_eq!(again, 0);
        assert_eq!(
            db.get_agent_run_steps("run-x")
                .unwrap()
                .iter()
                .filter(|step| step.step_type == "event")
                .count(),
            1,
            "reconcile must not duplicate markers on re-run"
        );
    }

    #[test]
    fn reconcile_also_cancels_paused_runs() {
        // P1-2: `paused` is an in-memory hold (suspended future + pause handle live in
        // process); after a restart it cannot resume, so startup reconcile must sweep
        // paused runs into cancelled just like running. (Also asserts that the run
        // status normalizer actually persists "paused" rather than falling back to
        // "running".)
        let db = temp_db();
        db.create_agent_run("run-p", None, "default").unwrap();
        db.update_agent_run_status("run-p", "paused", None).unwrap();
        assert_eq!(db.get_agent_run("run-p").unwrap().unwrap().status, "paused");

        let cancelled = db.mark_interrupted_agent_runs().unwrap();
        assert_eq!(cancelled, 1);

        let run = db.get_agent_run("run-p").unwrap().unwrap();
        assert_eq!(run.status, "cancelled");
        assert!(run.error.unwrap_or_default().contains("中断"));

        let steps = db.get_agent_run_steps("run-p").unwrap();
        assert!(
            steps
                .iter()
                .any(|step| step.step_type == "event" && step.summary.contains("异常中断")),
            "paused run should also get an interruption marker"
        );
    }

    #[test]
    fn paginate_run_timeline_orders_chronologically_and_pages() {
        // P1-3: build out-of-order entries with controlled timestamps; the helper
        // must sort chronologically and let the caller page through ALL of them in
        // order. (Red line: "latest N only / cannot replay the full run".)
        let mk = |id: &str, at: i64| RunTimelineEntry {
            kind: "step".to_string(),
            id: id.to_string(),
            at,
            finished_at: None,
            seq: 0,
            label: "x".to_string(),
            status: None,
            detail: serde_json::Value::Null,
        };
        let entries = vec![
            mk("c", 30),
            mk("a", 10),
            mk("d", 40),
            mk("b", 20),
            mk("e", 50),
        ];

        let (total, page1) = paginate_run_timeline(entries.clone(), 2, 0);
        assert_eq!(total, 5);
        assert_eq!(
            page1.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );

        // Paging through covers every entry in order, with no gap or overlap.
        let (_, page2) = paginate_run_timeline(entries.clone(), 2, 2);
        let (_, page3) = paginate_run_timeline(entries.clone(), 2, 4);
        let mut seen: Vec<&str> = Vec::new();
        seen.extend(page1.iter().map(|e| e.id.as_str()));
        seen.extend(page2.iter().map(|e| e.id.as_str()));
        seen.extend(page3.iter().map(|e| e.id.as_str()));
        assert_eq!(seen, ["a", "b", "c", "d", "e"]);

        // Offset past the end yields an empty page, not a panic.
        let (total_oob, oob) = paginate_run_timeline(entries, 2, 99);
        assert_eq!(total_oob, 5);
        assert!(oob.is_empty());
    }

    #[test]
    fn run_timeline_merges_all_run_sources() {
        // P1-3/P3-5: all run-scoped sources surface in one timeline, ordered by
        // time, with the real source record kept in `detail`.
        let db = temp_db();
        let session = db.create_session("s").unwrap();
        db.create_agent_run("run-tl", Some(&session.id), "default")
            .unwrap();

        db.append_agent_run_step(
            "run-tl",
            "assistant",
            "finished",
            "thinking",
            json!({ "prompt": "hi" }),
            json!({ "text": "hello" }),
        )
        .unwrap();

        db.log_agent_tool_audit_event(LogAgentToolAuditPayload {
            session_id: Some(session.id.clone()),
            run_id: "run-tl".to_string(),
            iteration: 1,
            tool_call_id: "tc-1".to_string(),
            tool_name: "read_file".to_string(),
            permission_mode: "default".to_string(),
            policy: "allow".to_string(),
            status: "allowed".to_string(),
            reason: "ok".to_string(),
        })
        .unwrap();

        db.log_model_usage_event(LogModelUsagePayload {
            session_id: Some(session.id.clone()),
            run_id: "run-tl".to_string(),
            iteration: 1,
            provider: "openai".to_string(),
            model: "gpt".to_string(),
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            source: "model_api_usage".to_string(),
        })
        .unwrap();

        let task = db
            .create_plan_task(&session.id, "build", None, Some("run-tl"), "user")
            .unwrap();
        db.record_task_verification(
            &task.id,
            Some("run-tl"),
            "build",
            "cargo build",
            Some(0),
            "passed",
            "ok",
            "",
            1_000,
            Some(1_200),
        )
        .unwrap();

        db.log_permission_decision(LogPermissionDecisionPayload {
            session_id: Some(session.id.clone()),
            run_id: "run-tl".to_string(),
            iteration: 1,
            tool_call_id: "tc-1".to_string(),
            subject: "command".to_string(),
            action: "run_command".to_string(),
            risk: "destructive".to_string(),
            mode: "default".to_string(),
            decision: "needs_confirm".to_string(),
            reason: "destructive command".to_string(),
            decided_by: "policy".to_string(),
        })
        .unwrap();

        let timeline = db.get_run_timeline("run-tl", 200, 0).unwrap();
        assert_eq!(timeline.total, 6, "all six run-scoped sources present");
        assert_eq!(timeline.entries.len(), 6);
        assert!(
            timeline.run.is_some(),
            "run metadata included for replay header"
        );

        let kinds: std::collections::HashSet<&str> =
            timeline.entries.iter().map(|e| e.kind.as_str()).collect();
        assert!(kinds.contains("step"));
        assert!(kinds.contains("tool"));
        assert!(kinds.contains("usage"));
        assert!(kinds.contains("verify"));
        assert!(
            kinds.contains("permission"),
            "P0-4 permission decision must be part of the event chain"
        );
        assert!(
            kinds.contains("plan_change"),
            "P3-5 plan changes must be part of the decision trail"
        );

        // The permission entry surfaces the decision via `status` and keeps the full
        // 谁批/为什么 in `detail`, not a fabricated summary.
        let perm = timeline
            .entries
            .iter()
            .find(|e| e.kind == "permission")
            .expect("permission entry present");
        assert_eq!(perm.status.as_deref(), Some("needs_confirm"));
        assert_eq!(perm.detail["decidedBy"], json!("policy"));
        assert_eq!(perm.detail["subject"], json!("command"));

        // Chronological invariant: `at` is non-decreasing across the merge.
        let ats: Vec<i64> = timeline.entries.iter().map(|e| e.at).collect();
        assert!(ats.windows(2).all(|w| w[0] <= w[1]), "ordered by time");

        // `detail` preserves the real source record, not a fabricated summary.
        let verify = timeline
            .entries
            .iter()
            .find(|e| e.kind == "verify")
            .expect("verify entry present");
        assert_eq!(verify.detail["command"], json!("cargo build"));
        assert_eq!(verify.detail["exitCode"], json!(0));

        let plan_change = timeline
            .entries
            .iter()
            .find(|e| e.kind == "plan_change")
            .expect("plan change entry present");
        assert_eq!(plan_change.detail["reason"], json!("创建计划任务。"));
        assert_eq!(plan_change.detail["subjectId"], json!(task.id));
    }

    #[test]
    fn run_timeline_empty_for_unknown_run() {
        let db = temp_db();
        let timeline = db.get_run_timeline("nope", 200, 0).unwrap();
        assert_eq!(timeline.total, 0);
        assert!(timeline.entries.is_empty());
        assert!(timeline.run.is_none());
    }

    #[test]
    fn task_verifications_can_be_listed_by_run() {
        // P2-13: DeliveryReport needs run-scoped verification rows, not only
        // per-task lookup.
        let db = temp_db();
        let session = db.create_session("verification-by-run").unwrap();
        db.create_agent_run("run-a", Some(&session.id), "default")
            .unwrap();
        db.create_agent_run("run-b", Some(&session.id), "default")
            .unwrap();
        let task_a = db
            .create_plan_task(&session.id, "verify A", None, Some("run-a"), "test")
            .unwrap();
        let task_b = db
            .create_plan_task(&session.id, "verify B", None, Some("run-b"), "test")
            .unwrap();

        db.record_task_verification(
            &task_a.id,
            Some("run-a"),
            "test",
            "cargo test a",
            Some(0),
            "passed",
            "ok",
            "",
            10,
            Some(11),
        )
        .unwrap();
        db.record_task_verification(
            &task_b.id,
            Some("run-b"),
            "test",
            "cargo test b",
            Some(0),
            "passed",
            "ok",
            "",
            20,
            Some(21),
        )
        .unwrap();

        let rows = db.list_task_verifications_by_run("run-a").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task_id, task_a.id);
        assert_eq!(rows[0].command, "cargo test a");
    }

    #[test]
    fn plan_change_events_capture_reason_before_after_and_timeline() {
        let db = temp_db();
        let session = db.create_session("plan audit").unwrap();
        db.create_agent_run("run-plan-audit", Some(&session.id), "default")
            .unwrap();

        let blank = db.log_plan_change_event(
            &session.id,
            Some("run-plan-audit"),
            "agent",
            "update",
            "plan_task",
            "task-1",
            "   ",
            Value::Null,
            Value::Null,
        );
        assert!(
            matches!(blank, Err(StorageError::Validation(_))),
            "plan mutations must explain why they happened"
        );
        let blank_create = db.create_plan_task_full_with_reason(
            &session.id,
            "不应落库",
            None,
            Some("run-plan-audit"),
            "agent",
            None,
            None,
            " ",
            "agent",
        );
        assert!(matches!(blank_create, Err(StorageError::Validation(_))));
        assert!(
            db.list_plan_tasks(&session.id).unwrap().is_empty(),
            "audit validation failure must not leave a silent plan mutation behind"
        );

        let task = db
            .create_plan_task_full_with_reason(
                &session.id,
                "实现审计",
                None,
                Some("run-plan-audit"),
                "agent",
                Some(&json!(["审计记录含 before/after/reason"])),
                None,
                "拆分第32张任务卡。",
                "agent",
            )
            .unwrap();
        let updated = db
            .update_plan_task_status_with_reason(
                &task.id,
                "running",
                Some("run-plan-audit"),
                "开始执行该任务。",
                "agent",
            )
            .unwrap();
        assert_eq!(updated.status, "running");
        db.set_active_plan_task_with_reason(
            &session.id,
            Some(&task.id),
            "切换到当前实现任务。",
            "agent",
        )
        .unwrap();
        db.set_active_plan_task_with_reason(
            &session.id,
            Some(&task.id),
            "重复激活不应写审计。",
            "agent",
        )
        .unwrap();

        let events = db
            .list_plan_change_events(&session.id, Some("run-plan-audit"), 20)
            .unwrap();
        assert_eq!(
            events.len(),
            3,
            "create/status/active are substantive changes; duplicate active is not"
        );
        assert!(events
            .iter()
            .any(|event| event.reason == "拆分第32张任务卡。"));
        assert!(events
            .iter()
            .any(|event| event.reason == "开始执行该任务。"));
        assert!(events
            .iter()
            .any(|event| event.reason == "切换到当前实现任务。"));

        let status_change = events
            .iter()
            .find(|event| event.action == "update_status")
            .expect("status audit");
        assert_eq!(status_change.before["status"], json!("pending"));
        assert_eq!(status_change.after["status"], json!("running"));
        assert_eq!(status_change.actor, "agent");

        let timeline = db.get_run_timeline("run-plan-audit", 50, 0).unwrap();
        let plan_change_count = timeline
            .entries
            .iter()
            .filter(|entry| entry.kind == "plan_change")
            .count();
        assert_eq!(plan_change_count, 3);
    }

    #[test]
    fn plan_task_details_update_records_audit_and_rejects_invalid_parents() {
        let db = temp_db();
        let session = db.create_session("plan edit").unwrap();
        db.create_agent_run("run-edit", Some(&session.id), "default")
            .unwrap();
        let parent = db
            .create_plan_task_full_with_reason(
                &session.id,
                "父任务",
                None,
                Some("run-edit"),
                "agent",
                None,
                None,
                "建立父任务。",
                "agent",
            )
            .unwrap();
        let child = db
            .create_plan_task_full_with_reason(
                &session.id,
                "子任务",
                Some(&parent.id),
                Some("run-edit"),
                "agent",
                None,
                None,
                "建立子任务。",
                "agent",
            )
            .unwrap();

        let cycle = db.update_plan_task_with_reason(
            PlanTaskPatch {
                id: parent.id.clone(),
                parent_id: Some(child.id.clone()),
                ..Default::default()
            },
            "制造循环不应成功。",
            "user",
        );
        assert!(matches!(cycle, Err(StorageError::Validation(_))));

        let updated = db
            .update_plan_task_with_reason(
                PlanTaskPatch {
                    id: child.id.clone(),
                    title: Some("编辑后的子任务".to_string()),
                    clear_parent_id: true,
                    clear_run_id: true,
                    position: Some(7),
                    acceptance_criteria: Some(json!(["用户可编辑验收标准"])),
                    verify: Some(json!({"command": "cargo test"})),
                    ..Default::default()
                },
                "用户编辑计划任务详情。",
                "user",
            )
            .unwrap();
        assert_eq!(updated.title, "编辑后的子任务");
        assert_eq!(updated.parent_id, None);
        assert_eq!(updated.run_id, None);
        assert_eq!(updated.position, 7);
        assert_eq!(updated.acceptance_criteria, json!(["用户可编辑验收标准"]));
        assert_eq!(updated.verify, json!({"command": "cargo test"}));

        let events = db
            .list_plan_change_events(&session.id, Some("run-edit"), 20)
            .unwrap();
        assert_eq!(events.len(), 3, "create/create/update should be audited");
        let update = events
            .iter()
            .find(|event| event.action == "update" && event.subject_id == child.id)
            .expect("details update audit");
        assert_eq!(update.reason, "用户编辑计划任务详情。");
        assert_eq!(update.actor, "user");
        assert_eq!(update.before["title"], json!("子任务"));
        assert_eq!(update.after["title"], json!("编辑后的子任务"));
        assert_eq!(update.after["parentId"], Value::Null);
        assert_eq!(update.after["runId"], Value::Null);

        db.update_plan_task_with_reason(
            PlanTaskPatch {
                id: child.id.clone(),
                ..Default::default()
            },
            "无变化不应写审计。",
            "user",
        )
        .unwrap();
        let after_noop = db
            .list_plan_change_events(&session.id, Some("run-edit"), 20)
            .unwrap();
        assert_eq!(after_noop.len(), 3);

        let other = db.create_session("other plan").unwrap();
        let other_parent = db
            .create_plan_task_full_with_reason(
                &other.id,
                "其他会话父任务",
                None,
                None,
                "agent",
                None,
                None,
                "建立其他会话任务。",
                "agent",
            )
            .unwrap();
        let bad_create = db.create_plan_task_full_with_reason(
            &session.id,
            "跨会话子任务",
            Some(&other_parent.id),
            None,
            "agent",
            None,
            None,
            "跨会话父任务不应创建。",
            "agent",
        );
        assert!(matches!(bad_create, Err(StorageError::Validation(_))));
        let cross_session = db.update_plan_task_with_reason(
            PlanTaskPatch {
                id: child.id,
                parent_id: Some(other_parent.id),
                ..Default::default()
            },
            "跨会话父任务不应成功。",
            "user",
        );
        assert!(matches!(cross_session, Err(StorageError::Validation(_))));
    }

    #[test]
    fn plan_task_run_id_must_belong_to_same_session() {
        let db = temp_db();
        let first = db.create_session("plan run first").unwrap();
        let second = db.create_session("plan run second").unwrap();
        db.create_agent_run("run-first", Some(&first.id), "default")
            .unwrap();
        db.create_agent_run("run-second", Some(&second.id), "default")
            .unwrap();
        db.create_agent_run("run-orphan", None, "default").unwrap();

        let task = db
            .create_plan_task(&first.id, "合法任务", None, Some("run-first"), "test")
            .unwrap();
        assert_eq!(task.run_id.as_deref(), Some("run-first"));

        let missing_create =
            db.create_plan_task(&first.id, "不存在 run", None, Some("missing-run"), "test");
        assert!(matches!(missing_create, Err(StorageError::NotFound(_))));
        let cross_create =
            db.create_plan_task(&first.id, "跨会话 run", None, Some("run-second"), "test");
        assert!(matches!(cross_create, Err(StorageError::Validation(_))));
        let orphan_create =
            db.create_plan_task(&first.id, "孤儿 run", None, Some("run-orphan"), "test");
        assert!(matches!(orphan_create, Err(StorageError::Validation(_))));

        let cross_status = db.update_plan_task_status(&task.id, "running", Some("run-second"));
        assert!(matches!(cross_status, Err(StorageError::Validation(_))));
        let missing_status = db.update_plan_task_status(&task.id, "running", Some("missing-run"));
        assert!(matches!(missing_status, Err(StorageError::NotFound(_))));
        let cross_patch = db.update_plan_task_with_reason(
            PlanTaskPatch {
                id: task.id.clone(),
                run_id: Some("run-second".to_string()),
                ..Default::default()
            },
            "跨会话 run 不应写入。",
            "user",
        );
        assert!(matches!(cross_patch, Err(StorageError::Validation(_))));

        let cleared = db
            .update_plan_task_with_reason(
                PlanTaskPatch {
                    id: task.id,
                    clear_run_id: true,
                    ..Default::default()
                },
                "允许用户清除 run 绑定。",
                "user",
            )
            .unwrap();
        assert_eq!(cleared.run_id, None);
    }

    #[test]
    fn plan_task_run_integrity_scan_and_repair_clears_legacy_bad_refs() {
        let db = temp_db();
        let first = db.create_session("legacy first").unwrap();
        let second = db.create_session("legacy second").unwrap();
        db.create_agent_run("legacy-run-second", Some(&second.id), "default")
            .unwrap();
        db.create_agent_run("legacy-run-orphan", None, "default")
            .unwrap();
        let now = now_ms();
        {
            let conn = db.conn.lock().expect("local db mutex poisoned");
            conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
            for (id, run_id, title, position) in [
                ("legacy-missing", "missing-run", "缺失 run", 1_i64),
                ("legacy-cross", "legacy-run-second", "跨会话 run", 2_i64),
                ("legacy-orphan", "legacy-run-orphan", "孤儿 run", 3_i64),
            ] {
                conn.execute(
                    "INSERT INTO plan_tasks(
                        id, session_id, run_id, parent_id, title, status, position, source,
                        created_at, updated_at, archived_at
                     ) VALUES (?1, ?2, ?3, NULL, ?4, 'pending', ?5, 'legacy', ?6, ?6, NULL)",
                    params![id, &first.id, run_id, title, position, now],
                )
                .unwrap();
            }
            conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        }

        let scan = db.scan_plan_task_run_integrity().unwrap();
        assert_eq!(scan.issue_count, 3);
        assert!(scan.issues.iter().any(|issue| issue.issue == "missing_run"));
        assert!(scan
            .issues
            .iter()
            .any(|issue| issue.issue == "cross_session"));
        assert!(scan
            .issues
            .iter()
            .any(|issue| issue.issue == "run_without_session"));

        let repair = db
            .repair_plan_task_run_integrity("M-13 清理历史坏 run_id。", "test")
            .unwrap();
        assert!(repair.repair_applied);
        assert_eq!(repair.repaired_count, 3);
        assert_eq!(db.scan_plan_task_run_integrity().unwrap().issue_count, 0);
        assert!(db
            .list_plan_tasks(&first.id)
            .unwrap()
            .iter()
            .all(|task| task.run_id.is_none()));
        let repair_events = db
            .list_plan_change_events(&first.id, None, 20)
            .unwrap()
            .into_iter()
            .filter(|event| event.action == "repair_integrity")
            .count();
        assert_eq!(repair_events, 3);
    }

    #[test]
    fn run_timeline_does_not_cross_runs() {
        // P1-3 (review point 4): the timeline is the event chain of ONE run. Events
        // from another run must never leak in — each source query is keyed by run_id.
        let db = temp_db();
        let session = db.create_session("s").unwrap();
        db.create_agent_run("run-a", Some(&session.id), "default")
            .unwrap();
        db.create_agent_run("run-b", Some(&session.id), "default")
            .unwrap();

        db.append_agent_run_step("run-a", "assistant", "finished", "a1", json!({}), json!({}))
            .unwrap();
        db.append_agent_run_step("run-b", "assistant", "finished", "b1", json!({}), json!({}))
            .unwrap();
        db.append_agent_run_step("run-b", "assistant", "finished", "b2", json!({}), json!({}))
            .unwrap();
        db.log_model_usage_event(LogModelUsagePayload {
            session_id: Some(session.id.clone()),
            run_id: "run-b".to_string(),
            iteration: 1,
            provider: "openai".to_string(),
            model: "gpt".to_string(),
            input_tokens: 1,
            output_tokens: 1,
            total_tokens: 2,
            source: "model_api_usage".to_string(),
        })
        .unwrap();

        let a = db.get_run_timeline("run-a", 200, 0).unwrap();
        assert_eq!(a.total, 1, "run-a sees only its own single event");
        assert!(a
            .entries
            .iter()
            .all(|e| e.detail["runId"] == json!("run-a")));

        let b = db.get_run_timeline("run-b", 200, 0).unwrap();
        assert_eq!(b.total, 3, "run-b sees only its own three events");
        assert!(b
            .entries
            .iter()
            .all(|e| e.detail["runId"] == json!("run-b")));
    }

    #[test]
    fn atlas_home_env_redirects_default_local_db() {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        let dir = std::env::temp_dir().join(format!("atlas_storage_home_{}", Uuid::new_v4()));
        std::env::set_var("ATLAS_HOME", &dir);

        assert_eq!(atlas_home().unwrap(), dir);
        let db = LocalDb::open_default().unwrap();
        assert_eq!(db.path(), dir.join("atlas.db"));
        assert!(db.path().exists());

        std::env::remove_var("ATLAS_HOME");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn init_migrates_legacy_project_and_message_columns() {
        let path = std::env::temp_dir().join(format!("atlas_legacy_{}.db", Uuid::new_v4()));
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE sessions (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE projects (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE messages (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );
                INSERT INTO sessions(id, title, created_at, updated_at)
                    VALUES ('legacy-session', 'Legacy Session', 10, 20);
                INSERT INTO projects(id, title, created_at, updated_at)
                    VALUES ('legacy-project', 'Legacy Project', 30, 40);
                INSERT INTO messages(id, session_id, role, content, created_at)
                    VALUES ('legacy-message', 'legacy-session', 'user', 'legacy content', 50);
                "#,
            )
            .unwrap();
        }

        let db = LocalDb::open(path).unwrap();
        let sessions = db.list_sessions().unwrap();
        assert_eq!(sessions[0].id, "legacy-session");
        assert_eq!(sessions[0].last_active_at, 20);
        assert!(sessions[0].archived_at.is_none());

        let projects = db.list_projects().unwrap();
        assert_eq!(projects[0].id, "legacy-project");
        assert_eq!(projects[0].kind, "folder");
        assert_eq!(projects[0].last_active_at, 40);

        let messages = db.get_messages("legacy-session").unwrap();
        assert_eq!(messages[0].metadata, json!({}));

        let export = db.export_local_data().unwrap();
        assert_eq!(export.schema_version, 8);
        assert!(export
            .messages
            .iter()
            .any(|message| message.id == "legacy-message"));
    }

    #[test]
    fn export_v8_round_trips_through_serde() {
        // P4-1: exported bundle remains JSON compatible after checkpoint
        // schema expansion; plugin package fields from v7 still round-trip.
        let db = temp_db();
        let row = ProviderCapabilitiesRow {
            provider_id: "anthropic".into(),
            model: "claude-opus-4-8".into(),
            vision: true,
            tool_calls: true,
            json_mode: false,
            max_context: 200_000,
            source: "builtin".into(),
            updated_at: 0,
        };
        db.upsert_provider_capabilities(&row).unwrap();
        let export = db.export_local_data().unwrap();
        let json = serde_json::to_string(&export).unwrap();
        let back: LocalDataExport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.schema_version, 8);
        assert!(back
            .provider_capabilities
            .iter()
            .any(|c| c.provider_id == "anthropic" && c.model == "claude-opus-4-8"));
        assert!(!back.capability_audit.is_empty());
        assert!(back.plan_change_events.is_empty());
        assert!(back.plugin_packages.is_empty());
        assert!(back.plugin_capability_events.is_empty());
    }

    #[test]
    fn import_v3_bundle_without_capability_fields_is_compatible() {
        // T32: a JSON that lacks provider_capabilities and capability_audit
        // (v3 / v4 era) must still parse with empty defaults.
        let v3_bundle = serde_json::json!({
            "schemaVersion": 3,
            "exportedAt": 0,
            "dbPath": "/tmp/old.db",
            "sessions": [],
            "messages": [],
            "projects": [],
            "agentRuns": [],
            "agentRunSteps": [],
            "agentToolAuditEvents": [],
            "modelUsageEvents": [],
            "planTasks": [],
            "artifacts": [],
            "memories": [],
            "profile": {
                "id": "default",
                "profile": {},
                "updated_at": 0,
            },
            "personalityProgress": {
                "id": "default",
                "progress": {},
                "updated_at": 0,
            },
            "appState": [],
            "activityEvents": [],
        });
        let parsed: LocalDataExport = serde_json::from_value(v3_bundle).unwrap();
        assert_eq!(parsed.schema_version, 3);
        assert!(parsed.provider_capabilities.is_empty());
        assert!(parsed.capability_audit.is_empty());
        assert!(parsed.plan_change_events.is_empty());
        assert!(parsed.plugin_packages.is_empty());
        assert!(parsed.plugin_capability_events.is_empty());
    }

    #[test]
    fn memory_crud_works() {
        let db = temp_db();
        let memory = db.add_memory("likes direct answers", "test").unwrap();
        assert!(memory.enabled);

        let updated = db.update_memory(&memory.id, None, Some(false)).unwrap();
        assert!(!updated.enabled);

        db.delete_memory(&memory.id).unwrap();
        assert!(db.list_memories().unwrap().is_empty());
    }

    #[test]
    fn deleting_last_session_creates_replacement() {
        let db = temp_db();
        let session = db.create_session("Test").unwrap();
        let sessions = db.delete_session(&session.id).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "新会话");
    }

    #[test]
    fn session_pin_archive_restore_round_trip() {
        let db = temp_db();
        let first = db.create_session("First").unwrap();
        let second = db.create_session("Second").unwrap();

        let pinned = db.set_session_pinned(&first.id, true).unwrap();
        assert!(pinned.pinned);
        let sessions = db.list_sessions().unwrap();
        assert_eq!(sessions[0].id, first.id);

        let active = db.archive_session(&first.id).unwrap();
        assert!(active.iter().all(|session| session.id != first.id));
        assert!(active.iter().any(|session| session.id == second.id));
        let archived = db.list_archived_sessions().unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, first.id);
        assert!(archived[0].archived_at.is_some());
        assert!(!archived[0].pinned);

        let restored = db.restore_session(&first.id).unwrap();
        assert_eq!(restored.id, first.id);
        assert!(restored.archived_at.is_none());
        assert!(db.list_archived_sessions().unwrap().is_empty());
        assert!(db
            .list_sessions()
            .unwrap()
            .iter()
            .any(|session| session.id == first.id));
    }

    #[test]
    fn archived_sessions_are_hidden_from_search() {
        let db = temp_db();
        let session = db.create_session("Hidden Review").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "unique archived search term".to_string(),
                created_at: None,
                metadata: json!({}),
            },
        )
        .unwrap();

        assert_eq!(db.search_sessions("unique archived").unwrap().len(), 1);
        db.archive_session(&session.id).unwrap();
        assert!(db.search_sessions("unique archived").unwrap().is_empty());
        let archived = db.search_archived_sessions("unique archived").unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, session.id);
        assert!(archived[0].archived_at.is_some());
    }

    #[test]
    fn model_usage_events_round_trip_and_filter_by_session() {
        let db = temp_db();
        db.log_model_usage_event(LogModelUsagePayload {
            session_id: Some("session-a".to_string()),
            run_id: "run-a".to_string(),
            iteration: 1,
            provider: "openai-compatible".to_string(),
            model: "deepseek-chat".to_string(),
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            source: "model_api_usage".to_string(),
        })
        .unwrap();
        db.log_model_usage_event(LogModelUsagePayload {
            session_id: Some("session-b".to_string()),
            run_id: "run-b".to_string(),
            iteration: 1,
            provider: "openai-compatible".to_string(),
            model: "deepseek-chat".to_string(),
            input_tokens: 3,
            output_tokens: 4,
            total_tokens: 7,
            source: "model_api_usage".to_string(),
        })
        .unwrap();
        db.log_model_usage_event(LogModelUsagePayload {
            session_id: Some("session-a".to_string()),
            run_id: "eval-run".to_string(),
            iteration: 1,
            provider: "eval".to_string(),
            model: "eval-model".to_string(),
            input_tokens: 100,
            output_tokens: 100,
            total_tokens: 200,
            source: "eval".to_string(),
        })
        .unwrap();

        let all = db.model_usage_summary(None).unwrap();
        assert_eq!(all.events, 2);
        assert_eq!(all.input_tokens, 13);
        assert_eq!(all.output_tokens, 9);
        assert_eq!(all.total_tokens, 22);
        assert_eq!(all.recent.len(), 2);

        let session_a = db.model_usage_summary(Some("session-a")).unwrap();
        assert_eq!(session_a.events, 1);
        assert_eq!(session_a.total_tokens, 15);
        assert_eq!(session_a.recent[0].run_id, "run-a");

        assert_eq!(db.model_usage_total_for_run("run-a").unwrap(), 15);
        assert_eq!(db.model_usage_total_for_session("session-a").unwrap(), 15);
        assert_eq!(db.model_usage_total_since(0).unwrap(), 22);
        assert_eq!(db.model_usage_total_for_run("eval-run").unwrap(), 0);
    }

    #[test]
    fn empty_session_title_defaults_to_chinese() {
        let db = temp_db();
        let session = db.create_session("  ").unwrap();
        assert_eq!(session.title, "新会话");
    }

    #[test]
    fn saves_messages_for_session() {
        let db = temp_db();
        let session = db.create_session("Test").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "hello".to_string(),
                created_at: None,
                metadata: json!({}),
            },
        )
        .unwrap();
        assert_eq!(db.get_messages(&session.id).unwrap().len(), 1);
    }

    #[test]
    fn project_sessions_keep_project_context() {
        let db = temp_db();
        let dir = std::env::temp_dir().join(format!("atlas_project_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = db
            .upsert_project("桌面项目", Some(&dir.to_string_lossy()), "folder")
            .unwrap();
        let session = db
            .create_session_for_project("项目对话", Some(&project.id))
            .unwrap();

        assert_eq!(session.project_id.as_deref(), Some(project.id.as_str()));
        assert_eq!(
            db.session_project_root(&session.id).unwrap().unwrap(),
            dir.canonicalize().unwrap()
        );
    }

    #[test]
    fn list_projects_filters_legacy_all_conversations_placeholder() {
        let db = temp_db();
        let visible = db
            .upsert_project(
                "真实项目",
                Some(&std::env::temp_dir().to_string_lossy()),
                "folder",
            )
            .unwrap();
        let hidden = db.upsert_project("全部对话", None, "virtual").unwrap();

        let projects = db.list_projects().unwrap();
        assert!(projects.iter().any(|project| project.id == visible.id));
        assert!(!projects.iter().any(|project| project.id == hidden.id));
    }

    #[test]
    fn conversation_history_report_reads_real_messages_and_range() {
        let db = temp_db();
        let recent = db.create_session("近期对话").unwrap();
        db.rename_session(&recent.id, "近期对话").unwrap();
        db.save_message(
            &recent.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "这周我们做到了什么地步？".to_string(),
                created_at: Some(now_ms() - 2 * 24 * 60 * 60 * 1000),
                metadata: json!({}),
            },
        )
        .unwrap();
        db.save_message(
            &recent.id,
            SaveMessagePayload {
                id: None,
                role: "assistant".to_string(),
                content: "已经完成桌面对话主路径。".to_string(),
                created_at: Some(now_ms() - 2 * 24 * 60 * 60 * 1000 + 1),
                metadata: json!({}),
            },
        )
        .unwrap();

        let old = db.create_session("旧对话").unwrap();
        db.rename_session(&old.id, "旧对话").unwrap();
        db.save_message(
            &old.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "三十天前的问题".to_string(),
                created_at: Some(now_ms() - 40 * 24 * 60 * 60 * 1000),
                metadata: json!({}),
            },
        )
        .unwrap();

        let weekly = db.conversation_history_report(Some("7d")).unwrap();
        assert_eq!(weekly.session_count, 1);
        assert_eq!(weekly.user_message_count, 1);
        assert!(weekly.report.contains("近期对话"));
        assert!(!weekly.report.contains("旧对话"));

        let all = db.conversation_history_report(Some("全部")).unwrap();
        assert_eq!(all.session_count, 2);
        assert!(all.report.contains("旧对话"));
    }

    #[test]
    fn user_messages_auto_title_unless_manually_renamed() {
        let db = temp_db();
        let session = db.create_session("新会话").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "帮我做一个桌面网页，带流体动画".to_string(),
                created_at: None,
                metadata: json!({}),
            },
        )
        .unwrap();
        let titled = db.get_session(&session.id).unwrap();
        assert!(titled.title.contains("桌面网页"));
        assert!(!titled.title_is_manual);

        let renamed = db.rename_session(&session.id, "手动标题").unwrap();
        assert!(renamed.title_is_manual);
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "这一句不应该覆盖手动标题".to_string(),
                created_at: None,
                metadata: json!({}),
            },
        )
        .unwrap();
        assert_eq!(db.get_session(&session.id).unwrap().title, "手动标题");
    }

    #[test]
    fn clear_session_context_records_cutoff_without_deleting_history() {
        let db = temp_db();
        let session = db.create_session("Test").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "保留在历史里的旧消息".to_string(),
                created_at: None,
                metadata: json!({}),
            },
        )
        .unwrap();
        let cutoff = db.clear_session_context(&session.id).unwrap();

        assert_eq!(
            db.session_context_clear_after(&session.id).unwrap(),
            Some(cutoff)
        );
        assert_eq!(db.get_messages(&session.id).unwrap().len(), 1);
        assert!(db.get_session_summary(&session.id).unwrap().is_none());
    }

    #[test]
    fn app_state_round_trips() {
        let db = temp_db();
        db.set_app_state("preview_level", json!("L3")).unwrap();
        assert_eq!(
            db.get_app_state("preview_level").unwrap(),
            Some(json!("L3"))
        );
    }

    #[test]
    fn session_summary_compacts_older_messages() {
        let db = temp_db();
        let session = db.create_session("Long chat").unwrap();
        for index in 0..18 {
            db.save_message(
                &session.id,
                SaveMessagePayload {
                    id: None,
                    role: if index % 2 == 0 {
                        "user".to_string()
                    } else {
                        "assistant".to_string()
                    },
                    content: format!("message number {index} with enough content to summarize"),
                    created_at: None,
                    metadata: json!({}),
                },
            )
            .unwrap();
        }

        let summary = db.summarize_session(&session.id).unwrap();
        assert_eq!(summary.source_message_count, 18);
        assert!(summary.summary.contains("压缩摘要"));
        assert!(db.get_session_summary(&session.id).unwrap().is_some());
    }

    #[test]
    fn export_local_data_contains_core_records() {
        let db = temp_db();
        let session = db.create_session("Export").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "export me".to_string(),
                created_at: None,
                metadata: json!({ "kind": "test" }),
            },
        )
        .unwrap();
        db.add_memory("remember this", "test").unwrap();
        db.set_app_state("preview_level", json!("L3")).unwrap();
        let archived = db.create_session("Archived Export").unwrap();
        db.save_message(
            &archived.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "keep archived message".to_string(),
                created_at: None,
                metadata: json!({ "kind": "archived" }),
            },
        )
        .unwrap();
        db.archive_session(&archived.id).unwrap();

        let export = db.export_local_data().unwrap();
        assert_eq!(export.schema_version, 8);
        assert_eq!(export.sessions.len(), 2);
        assert!(export
            .sessions
            .iter()
            .any(|item| item.id == archived.id && item.archived_at.is_some()));
        assert_eq!(export.messages.len(), 2);
        assert!(export
            .messages
            .iter()
            .any(|item| item.session_id == archived.id));
        assert_eq!(export.memories.len(), 1);
        assert!(export
            .app_state
            .iter()
            .any(|item| item.key == "preview_level"));
        assert!(export.db_path.ends_with(".db"));
    }

    #[test]
    fn health_counts_core_records() {
        let db = temp_db();
        let session = db.create_session("Health").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "health check".to_string(),
                created_at: None,
                metadata: json!({}),
            },
        )
        .unwrap();
        db.add_memory("health memory", "test").unwrap();
        db.set_app_state("preview_level", json!("L2")).unwrap();

        let health = db.health().unwrap();
        assert!(health.ok);
        assert_eq!(health.sessions, 1);
        assert_eq!(health.messages, 1);
        assert_eq!(health.memories, 1);
        assert_eq!(health.app_state, 1);
        assert!(health.db_path.ends_with(".db"));
    }

    #[test]
    fn reset_local_data_preserves_config_and_recreates_session() {
        let db = temp_db();
        let session = db.create_session("Reset").unwrap();
        db.save_message(
            &session.id,
            SaveMessagePayload {
                id: None,
                role: "user".to_string(),
                content: "remove me".to_string(),
                created_at: None,
                metadata: json!({}),
            },
        )
        .unwrap();
        db.add_memory("temporary", "test").unwrap();

        let summary = db
            .reset_local_data(ResetLocalDataOptions {
                sessions: true,
                memories: true,
                ..Default::default()
            })
            .unwrap();

        assert!(summary.preserved_config);
        assert!(summary.reset_scopes.contains(&"sessions".to_string()));
        assert!(summary.reset_scopes.contains(&"memories".to_string()));
        assert_eq!(db.list_memories().unwrap().len(), 0);
        assert_eq!(db.list_sessions().unwrap().len(), 1);
        assert!(summary.replacement_session.is_some());
    }

    #[test]
    fn agent_tool_audit_round_trips_without_arguments() {
        let db = temp_db();
        db.log_agent_tool_audit_event(LogAgentToolAuditPayload {
            session_id: Some("session-1".to_string()),
            run_id: "run-1".to_string(),
            iteration: 2,
            tool_call_id: "call-1".to_string(),
            tool_name: "read_file".to_string(),
            permission_mode: "workspace".to_string(),
            policy: "no_mutation".to_string(),
            status: "blocked".to_string(),
            reason: "policy_blocks_tool_metadata".to_string(),
        })
        .unwrap();

        let records = db
            .recent_agent_tool_audit_events(Some("session-1"), 10)
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].run_id, "run-1");
        assert_eq!(records[0].iteration, 2);
        assert_eq!(records[0].tool_name, "read_file");
        assert_eq!(records[0].permission_mode, "default");
        assert_eq!(records[0].policy, "default");
        assert_eq!(records[0].status, "blocked");
        assert_eq!(records[0].reason, "policy_blocks_tool_metadata");
    }

    #[test]
    fn agent_tool_audit_limits_and_orders_recent_entries() {
        let db = temp_db();
        for index in 0..3 {
            db.log_agent_tool_audit_event(LogAgentToolAuditPayload {
                session_id: Some("session-1".to_string()),
                run_id: format!("run-{index}"),
                iteration: 1,
                tool_call_id: format!("call-{index}"),
                tool_name: "read_file".to_string(),
                permission_mode: "plan".to_string(),
                policy: "plan".to_string(),
                status: "allowed".to_string(),
                reason: "policy_allowed".to_string(),
            })
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        let records = db.recent_agent_tool_audit_events(None, 2).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].run_id, "run-2");
        assert_eq!(records[1].run_id, "run-1");
        assert!(records
            .iter()
            .all(|record| record.reason != "{\"path\":\"secret.txt\"}"));
    }

    #[test]
    fn agent_run_steps_round_trip_and_update_status() {
        let db = temp_db();
        let session = db.create_session("Runtime").unwrap();
        let run = db
            .create_agent_run("run-test", Some(&session.id), "workspace")
            .unwrap();
        assert_eq!(run.permission_mode, "default");

        db.append_agent_run_step(
            &run.id,
            "tool_call",
            "running",
            "调用工具：read_file",
            json!({ "toolName": "read_file" }),
            json!({}),
        )
        .unwrap();
        db.append_agent_run_step(
            &run.id,
            "response",
            "finished",
            "生成最终回复。",
            json!({}),
            json!({ "content": "done" }),
        )
        .unwrap();
        db.update_agent_run_status(&run.id, "completed", None)
            .unwrap();

        let runs = db.recent_agent_runs(Some(&session.id), 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "finished");
        let steps = db.get_agent_run_steps(&run.id).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].step_index, 1);
        assert_eq!(steps[1].step_type, "response");

        db.update_agent_run_status(&run.id, "blocked", Some("budget hard limit"))
            .unwrap();
        let blocked = db.get_agent_run(&run.id).unwrap().unwrap();
        assert_eq!(blocked.status, "blocked");
        assert_eq!(blocked.error.as_deref(), Some("budget hard limit"));
    }

    #[test]
    fn finishing_latest_tool_call_step_clears_running_status() {
        let db = temp_db();
        let session = db.create_session("Runtime").unwrap();
        let run = db
            .create_agent_run("run-tool-finish", Some(&session.id), "full_access")
            .unwrap();
        db.append_agent_run_step(
            &run.id,
            "tool_call",
            "running",
            "调用工具：write_file",
            json!({ "toolName": "write_file" }),
            json!({}),
        )
        .unwrap();

        assert!(db
            .finish_latest_agent_tool_call_step(&run.id, json!({ "status": "success" }))
            .unwrap());

        let steps = db.get_agent_run_steps(&run.id).unwrap();
        assert_eq!(steps[0].status, "finished");
        assert!(steps[0].finished_at.is_some());
        assert_eq!(steps[0].output["status"], "success");
    }

    #[test]
    fn interrupted_running_agent_runs_are_cancelled_on_reopen() {
        let db = temp_db();
        let session = db.create_session("Runtime").unwrap();
        let running = db
            .create_agent_run("run-stale", Some(&session.id), "full_access")
            .unwrap();
        db.append_agent_run_step(
            &running.id,
            "tool_call",
            "running",
            "调用工具：run_command",
            json!({ "toolName": "run_command" }),
            json!({}),
        )
        .unwrap();
        let finished = db
            .create_agent_run("run-done", Some(&session.id), "full_access")
            .unwrap();
        db.append_agent_run_step(
            &finished.id,
            "tool_call",
            "running",
            "调用工具：write_file",
            json!({ "toolName": "write_file" }),
            json!({}),
        )
        .unwrap();
        db.update_agent_run_status(&finished.id, "finished", None)
            .unwrap();

        assert_eq!(db.mark_interrupted_agent_runs().unwrap(), 1);

        let stale = db.get_agent_run(&running.id).unwrap().unwrap();
        let done = db.get_agent_run(&finished.id).unwrap().unwrap();
        assert_eq!(stale.status, "cancelled");
        assert!(stale.error.unwrap().contains("运行时重启"));
        assert_eq!(done.status, "finished");
        assert_eq!(
            db.get_agent_run_steps(&running.id).unwrap()[0].status,
            "cancelled"
        );
        assert_eq!(
            db.get_agent_run_steps(&finished.id).unwrap()[0].status,
            "finished"
        );
    }

    #[test]
    fn finds_run_id_for_pending_command_step() {
        let db = temp_db();
        let session = db.create_session("Runtime").unwrap();
        let run = db
            .create_agent_run("run-command", Some(&session.id), "default")
            .unwrap();
        let pending = db
            .prepare_command(
                "Write-Output atlas".to_string(),
                ".".to_string(),
                "测试命令".to_string(),
                "powershell".to_string(),
            )
            .unwrap();

        db.append_agent_run_step(
            &run.id,
            "approval",
            "finished",
            "命令预览已准备，尚未运行。",
            json!({}),
            json!({
                "data": {
                    "pendingCommand": {
                        "id": pending.id.clone(),
                        "command": pending.command.clone(),
                        "cwd": pending.cwd.clone()
                    }
                }
            }),
        )
        .unwrap();

        let found = db.find_run_id_for_pending_command(&pending.id).unwrap();
        assert_eq!(found.as_deref(), Some(run.id.as_str()));
    }

    #[test]
    fn memories_dedupe_and_track_usage() {
        let db = temp_db();
        let first = db
            .add_memory("  Reply with the conclusion first.  ", "manual")
            .unwrap();
        let duplicate = db
            .add_memory("reply   with the conclusion first.", "agent")
            .unwrap();
        assert_eq!(first.id, duplicate.id);
        assert_eq!(db.list_memories().unwrap().len(), 1);
        assert_eq!(duplicate.quality, "confirmed");
        assert!(duplicate.enabled);

        db.mark_enabled_memories_used().unwrap();
        let memories = db.list_memories().unwrap();
        assert_eq!(memories[0].use_count, 1);
        assert!(memories[0].last_used_at.is_some());
        assert!(memories[0].confidence >= 0.9);
    }

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    #[test]
    fn decay_lowers_confidence_for_unreconfirmed_memories() {
        let db = temp_db();
        let m = db.add_memory("闲聊：今天天气不错", "agent").unwrap();
        assert_eq!(m.confidence, 1.0);
        let future = m.updated_at + 8 * DAY_MS;

        let changed = db.decay_memories(future, 7 * DAY_MS, 0.3).unwrap();
        assert_eq!(changed, 1);
        assert!((db.list_memories().unwrap()[0].confidence - 0.7).abs() < 1e-9);

        // Repeated maintenance passes keep lowering, clamped at 0.
        db.decay_memories(future, 7 * DAY_MS, 0.3).unwrap();
        db.decay_memories(future, 7 * DAY_MS, 0.3).unwrap();
        db.decay_memories(future, 7 * DAY_MS, 0.3).unwrap();
        assert_eq!(db.list_memories().unwrap()[0].confidence, 0.0);
    }

    #[test]
    fn decay_skips_recently_reinforced_memories() {
        let db = temp_db();
        let m = db.add_memory("稳定规则：先给结论", "manual").unwrap();
        // Within the idle window → not decayed.
        let changed = db
            .decay_memories(m.updated_at + DAY_MS, 7 * DAY_MS, 0.3)
            .unwrap();
        assert_eq!(changed, 0);
        assert_eq!(db.list_memories().unwrap()[0].confidence, 1.0);
    }

    #[test]
    fn purge_low_confidence_soft_disables_and_is_recoverable() {
        let db = temp_db();
        let m = db.add_memory("过期闲聊", "agent").unwrap();
        // Drive confidence below the floor.
        db.decay_memories(m.updated_at + 8 * DAY_MS, 7 * DAY_MS, 0.9)
            .unwrap();

        let purged = db.purge_low_confidence_memories(0.2).unwrap();
        assert_eq!(purged, 1);
        let after = db.list_memories().unwrap();
        // Soft-delete: row kept, only disabled + marked decayed.
        assert_eq!(after.len(), 1);
        assert!(!after[0].enabled);
        assert_eq!(after[0].quality, "decayed");

        // Recoverable: re-enabling restores it to the injection set.
        let restored = db.update_memory(&m.id, None, Some(true)).unwrap();
        assert!(restored.enabled);
        assert_eq!(restored.quality, "confirmed");
        assert_eq!(restored.confidence, 1.0);

        // Recovery must survive the next maintenance pass instead of being
        // immediately soft-deleted again because confidence stayed below floor.
        db.maintain_memories().unwrap();
        let after_maintain = db.list_memories().unwrap();
        assert!(after_maintain[0].enabled);
        assert_eq!(after_maintain[0].quality, "confirmed");
    }

    #[test]
    fn reconfirming_memory_restores_confidence() {
        let db = temp_db();
        let m = db.add_memory("先给结论再展开", "agent").unwrap();
        db.decay_memories(m.updated_at + 8 * DAY_MS, 7 * DAY_MS, 0.7)
            .unwrap();
        assert!((db.list_memories().unwrap()[0].confidence - 0.3).abs() < 1e-9);

        // Re-confirmation (dedupe re-add) lifts confidence back to 1.0.
        let again = db.add_memory("先给结论再展开", "manual").unwrap();
        assert_eq!(again.id, m.id);
        assert_eq!(db.list_memories().unwrap()[0].confidence, 1.0);
    }

    #[test]
    fn injection_use_does_not_touch_decay_clock() {
        let db = temp_db();
        let m = db.add_memory("注入不算再确认", "agent").unwrap();
        let updated_at_before = m.updated_at;

        // Being injected bumps usage telemetry but must not reset `updated_at`,
        // otherwise stale chitchat would never decay.
        db.mark_enabled_memories_used().unwrap();
        let after = db.list_memories().unwrap();
        assert_eq!(after[0].use_count, 1);
        assert!(after[0].last_used_at.is_some());
        assert_eq!(after[0].updated_at, updated_at_before);
    }

    #[test]
    fn plan_tasks_round_trip_status_and_archive() {
        let db = temp_db();
        let session = db.create_session("Plan tasks").unwrap();
        db.create_agent_run("run-1", Some(&session.id), "default")
            .unwrap();
        db.create_agent_run("run-2", Some(&session.id), "default")
            .unwrap();
        let parent = db
            .create_plan_task(&session.id, "补运行时间线", None, None, "plan_mode")
            .unwrap();
        let child = db
            .create_plan_task(
                &session.id,
                "显示真实步骤",
                Some(&parent.id),
                Some("run-1"),
                "manual",
            )
            .unwrap();

        let tasks = db.list_plan_tasks(&session.id).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, parent.id);
        assert_eq!(tasks[1].parent_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(child.position, 2);

        let updated = db
            .update_plan_task_status(&parent.id, "completed", Some("run-2"))
            .unwrap();
        assert_eq!(updated.status, "done");
        assert_eq!(updated.run_id.as_deref(), Some("run-2"));

        db.archive_plan_task(&parent.id).unwrap();
        let remaining = db.list_plan_tasks(&session.id).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, child.id);
    }

    #[test]
    fn artifacts_round_trip_with_diff_metadata() {
        let db = temp_db();
        let session = db.create_session("Artifacts").unwrap();
        db.create_agent_run("run-file", Some(&session.id), "default")
            .unwrap();
        let artifact = db
            .record_artifact(RecordArtifactPayload {
                session_id: Some(session.id.clone()),
                run_id: Some("run-file".to_string()),
                kind: "file".to_string(),
                title: "demo.txt".to_string(),
                path: Some("C:/tmp/demo.txt".to_string()),
                operation: "overwrite".to_string(),
                status: "written".to_string(),
                summary: "写入 demo.txt".to_string(),
                metadata: json!({ "diff": "-old\n+new" }),
            })
            .unwrap();

        let records = db.recent_artifacts(Some(&session.id), 10).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, artifact.id);
        assert_eq!(records[0].status, "written");
        assert_eq!(records[0].metadata["diff"], "-old\n+new");
        assert!(db.recent_artifacts(Some("other"), 10).unwrap().is_empty());
    }

    #[test]
    fn artifact_scope_validation_keeps_run_and_session_bound() {
        let db = temp_db();
        let first = db.create_session("First").unwrap();
        let second = db.create_session("Second").unwrap();
        db.create_agent_run("run-file", Some(&first.id), "default")
            .unwrap();

        let explicit = db
            .validate_artifact_scope(Some(&first.id), Some("run-file"))
            .unwrap();
        assert_eq!(
            explicit,
            (Some(first.id.clone()), Some("run-file".to_string()))
        );

        let inferred = db.validate_artifact_scope(None, Some("run-file")).unwrap();
        assert_eq!(
            inferred,
            (Some(first.id.clone()), Some("run-file".to_string()))
        );

        let mismatch = db
            .validate_artifact_scope(Some(&second.id), Some("run-file"))
            .unwrap_err();
        assert!(matches!(mismatch, StorageError::Validation(_)));

        let missing_session = db.validate_artifact_scope(Some("missing-session"), None);
        assert!(matches!(missing_session, Err(StorageError::NotFound(_))));

        let missing_run = db.validate_artifact_scope(Some(&first.id), Some("missing-run"));
        assert!(matches!(missing_run, Err(StorageError::NotFound(_))));
    }

    #[test]
    fn artifact_scope_validation_rejects_orphan_run_session_pairing() {
        let db = temp_db();
        let session = db.create_session("Artifacts").unwrap();
        db.create_agent_run("run-orphan", None, "default").unwrap();

        let run_only = db
            .validate_artifact_scope(None, Some("run-orphan"))
            .unwrap();
        assert_eq!(run_only, (None, Some("run-orphan".to_string())));

        let paired = db.validate_artifact_scope(Some(&session.id), Some("run-orphan"));
        assert!(matches!(paired, Err(StorageError::Validation(_))));
    }

    #[test]
    fn export_includes_agent_records_and_redacts_secret_app_state() {
        let db = temp_db();
        let project = db
            .upsert_project(
                "Export project",
                Some(&std::env::current_dir().unwrap().to_string_lossy()),
                "folder",
            )
            .unwrap();
        let session = db
            .create_session_for_project("Export session", Some(&project.id))
            .unwrap();
        db.create_agent_run("run-export", Some(&session.id), "default")
            .unwrap();
        db.append_agent_run_step(
            "run-export",
            "tool_call",
            "finished",
            "read",
            json!({ "path": "Cargo.toml" }),
            json!({ "ok": true }),
        )
        .unwrap();
        db.log_agent_tool_audit_event(LogAgentToolAuditPayload {
            session_id: Some(session.id.clone()),
            run_id: "run-export".to_string(),
            iteration: 1,
            tool_call_id: "call-1".to_string(),
            tool_name: "read_file".to_string(),
            permission_mode: "default".to_string(),
            policy: "allow".to_string(),
            status: "ok".to_string(),
            reason: "test".to_string(),
        })
        .unwrap();
        db.log_model_usage_event(LogModelUsagePayload {
            session_id: Some(session.id.clone()),
            run_id: "run-export".to_string(),
            iteration: 1,
            provider: "openai".to_string(),
            model: "test-model".to_string(),
            input_tokens: 1,
            output_tokens: 2,
            total_tokens: 3,
            source: "model_api_usage".to_string(),
        })
        .unwrap();
        db.create_plan_task(
            &session.id,
            "Do one thing",
            None,
            Some("run-export"),
            "test",
        )
        .unwrap();
        db.record_artifact(RecordArtifactPayload {
            session_id: Some(session.id.clone()),
            run_id: Some("run-export".to_string()),
            kind: "file".to_string(),
            title: "demo.txt".to_string(),
            path: Some("demo.txt".to_string()),
            operation: "write".to_string(),
            status: "written".to_string(),
            summary: "demo".to_string(),
            metadata: json!({ "diff": "-a\n+b" }),
        })
        .unwrap();
        db.set_app_state(
            "mcp_servers",
            json!([{
                "name": "Secret MCP",
                "authToken": "mcp-secret-token",
                "headers": [{ "key": "Authorization", "value": "Bearer mcp-secret-token" }]
            }]),
        )
        .unwrap();
        db.set_app_state("provider_field:openai:api_key", json!("sk-secret"))
            .unwrap();

        let export = db.export_local_data().unwrap();
        let export_text = serde_json::to_string(&export).unwrap();

        assert_eq!(export.schema_version, 8);
        assert_eq!(export.projects.len(), 1);
        assert_eq!(export.agent_runs.len(), 1);
        assert_eq!(export.agent_run_steps.len(), 1);
        assert_eq!(export.agent_tool_audit_events.len(), 1);
        assert_eq!(export.model_usage_events.len(), 1);
        assert_eq!(export.plan_tasks.len(), 1);
        assert_eq!(export.plan_change_events.len(), 1);
        assert_eq!(export.artifacts.len(), 1);
        assert!(!export_text.contains("mcp-secret-token"));
        assert!(!export_text.contains("sk-secret"));
        assert!(export_text.contains("***REDACTED***"));
    }

    #[test]
    fn reset_sessions_removes_agent_project_and_artifact_records() {
        let db = temp_db();
        let project = db
            .upsert_project(
                "Reset project",
                Some(&std::env::current_dir().unwrap().to_string_lossy()),
                "folder",
            )
            .unwrap();
        let session = db
            .create_session_for_project("Reset session", Some(&project.id))
            .unwrap();
        db.create_agent_run("run-reset", Some(&session.id), "default")
            .unwrap();
        db.append_agent_run_step(
            "run-reset",
            "response",
            "finished",
            "done",
            json!({}),
            json!({}),
        )
        .unwrap();
        db.create_plan_task(&session.id, "Reset task", None, Some("run-reset"), "test")
            .unwrap();
        db.record_artifact(RecordArtifactPayload {
            session_id: Some(session.id.clone()),
            run_id: Some("run-reset".to_string()),
            kind: "file".to_string(),
            title: "reset.txt".to_string(),
            path: Some("reset.txt".to_string()),
            operation: "write".to_string(),
            status: "written".to_string(),
            summary: "reset".to_string(),
            metadata: json!({}),
        })
        .unwrap();

        db.reset_local_data(ResetLocalDataOptions {
            sessions: true,
            ..Default::default()
        })
        .unwrap();

        let export = db.export_local_data().unwrap();
        assert_eq!(export.projects.len(), 0);
        assert_eq!(export.agent_runs.len(), 0);
        assert_eq!(export.agent_run_steps.len(), 0);
        assert_eq!(export.plan_tasks.len(), 0);
        assert_eq!(export.plan_change_events.len(), 0);
        assert_eq!(export.artifacts.len(), 0);
        assert_eq!(export.sessions.len(), 1);
    }

    #[test]
    fn file_write_diff_shows_changed_lines() {
        let diff = file_write_diff(Some("alpha\nold\nomega"), "alpha\nnew\nomega");
        assert!(diff.contains("--- 当前文件"));
        assert!(diff.contains("+++ 新内容"));
        assert!(diff.contains("-old"));
        assert!(diff.contains("+new"));
    }
}
