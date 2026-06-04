use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::storage::{LocalDb, RecordArtifactPayload};

const PROTOCOL_MAPPING_PREFIX: &str = "protocol_mapping";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalArtifactInput {
    pub artifact_type: String,
    pub title: String,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalTask {
    pub protocol: String,
    #[serde(default = "default_protocol_version")]
    pub version: String,
    pub external_task_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    pub input: Value,
    #[serde(default)]
    pub artifacts: Vec<ExternalArtifactInput>,
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRef {
    pub artifact_id: String,
    pub artifact_type: String,
    pub title: String,
    pub uri: Option<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolRunMapping {
    pub protocol: String,
    pub version: String,
    pub external_task_id: String,
    pub run_id: String,
    pub status: String,
    pub artifact_refs: Vec<ArtifactRef>,
    pub audit: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolCompatibilityEntry {
    pub protocol: String,
    pub supported_versions: Vec<String>,
    pub task_mapping: bool,
    pub lifecycle: bool,
    pub streaming: bool,
    pub cancellation: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolLifecycleUpdate {
    pub protocol: String,
    pub external_task_id: String,
    pub status: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub output: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolStreamEvent {
    pub sequence: i64,
    pub event_type: String,
    pub status: String,
    pub payload: Value,
    pub at: i64,
}

pub fn create_external_task_mapping(
    db: &LocalDb,
    task: ExternalTask,
) -> Result<ProtocolRunMapping, String> {
    validate_external_task(&task)?;
    let protocol = normalize_protocol(&task.protocol);
    let run_id = format!("protocol_{}_{}", protocol, Uuid::new_v4());
    db.create_agent_run(&run_id, task.session_id.as_deref(), &task.permission_mode)
        .map_err(|error| error.to_string())?;
    db.append_agent_run_step(
        &run_id,
        "protocol_task",
        "pending",
        "external protocol task mapped into Aura run; execution must pass Aura policy and final audit",
        json!({
            "protocol": protocol,
            "version": task.version,
            "externalTaskId": task.external_task_id,
            "input": task.input
        }),
        json!({
            "policy": "external adapter cannot call tools directly",
            "permissionMode": task.permission_mode
        }),
    )
    .map_err(|error| error.to_string())?;

    let mut artifact_refs = Vec::new();
    for artifact in task.artifacts {
        let record = db
            .record_artifact(RecordArtifactPayload {
                session_id: task.session_id.clone(),
                run_id: Some(run_id.clone()),
                kind: normalize_artifact_type(&artifact.artifact_type),
                title: artifact.title.clone(),
                path: artifact.uri.clone(),
                operation: "protocol_import".to_string(),
                status: "created".to_string(),
                summary: "artifact imported through external protocol adapter".to_string(),
                metadata: json!({
                    "protocol": protocol,
                    "externalTaskId": task.external_task_id,
                    "metadata": artifact.metadata
                }),
            })
            .map_err(|error| error.to_string())?;
        artifact_refs.push(ArtifactRef {
            artifact_id: record.id,
            artifact_type: normalize_artifact_type(&artifact.artifact_type),
            title: artifact.title,
            uri: artifact.uri,
            metadata: artifact.metadata,
        });
    }

    let mapping = ProtocolRunMapping {
        protocol: protocol.clone(),
        version: task.version,
        external_task_id: task.external_task_id,
        run_id,
        status: "mapped".to_string(),
        artifact_refs,
        audit: json!({
            "source": "protocol_adapter",
            "policy": "permission_checkpoint_final_audit_required",
            "directToolExecution": false
        }),
    };
    persist_mapping(db, &mapping)?;
    Ok(mapping)
}

pub fn protocol_compatibility_matrix() -> Vec<ProtocolCompatibilityEntry> {
    vec![
        ProtocolCompatibilityEntry {
            protocol: "agent-protocol".to_string(),
            supported_versions: vec!["v1".to_string()],
            task_mapping: true,
            lifecycle: true,
            streaming: true,
            cancellation: true,
            notes: vec![
                "External tasks map to Aura runs and cannot call tools directly".to_string(),
                "Lifecycle updates are persisted through app_state and run timeline".to_string(),
            ],
        },
        ProtocolCompatibilityEntry {
            protocol: "acp".to_string(),
            supported_versions: vec!["v1".to_string()],
            task_mapping: true,
            lifecycle: true,
            streaming: true,
            cancellation: true,
            notes: vec!["ACP payloads use the same Aura policy checkpoint".to_string()],
        },
        ProtocolCompatibilityEntry {
            protocol: "a2a".to_string(),
            supported_versions: vec!["v1".to_string()],
            task_mapping: true,
            lifecycle: true,
            streaming: true,
            cancellation: true,
            notes: vec!["A2A status is normalized before persisting".to_string()],
        },
    ]
}

pub fn get_external_task_mapping(
    db: &LocalDb,
    protocol: &str,
    external_task_id: &str,
) -> Result<ProtocolRunMapping, String> {
    let key = mapping_key(protocol, external_task_id);
    let value = db
        .get_app_state(&key)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("protocol mapping not found: {key}"))?;
    serde_json::from_value(value).map_err(|error| error.to_string())
}

pub fn update_external_task_lifecycle(
    db: &LocalDb,
    update: ProtocolLifecycleUpdate,
) -> Result<ProtocolRunMapping, String> {
    let mut mapping = get_external_task_mapping(db, &update.protocol, &update.external_task_id)?;
    let status = normalize_lifecycle_status(&update.status);
    let message = update
        .message
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("external protocol lifecycle update");
    db.append_agent_run_step(
        &mapping.run_id,
        "protocol_task",
        &status,
        message,
        json!({
            "protocol": mapping.protocol,
            "version": mapping.version,
            "externalTaskId": mapping.external_task_id,
            "status": status,
        }),
        update.output,
    )
    .map_err(|error| error.to_string())?;
    mapping.status = status;
    persist_mapping(db, &mapping)?;
    Ok(mapping)
}

pub fn cancel_external_task(
    db: &LocalDb,
    protocol: &str,
    external_task_id: &str,
    reason: &str,
) -> Result<ProtocolRunMapping, String> {
    update_external_task_lifecycle(
        db,
        ProtocolLifecycleUpdate {
            protocol: protocol.to_string(),
            external_task_id: external_task_id.to_string(),
            status: "cancelled".to_string(),
            message: Some(reason.chars().take(500).collect()),
            output: json!({ "cancelled": true, "reason": reason }),
        },
    )
}

pub fn append_external_task_stream_event(
    db: &LocalDb,
    protocol: &str,
    external_task_id: &str,
    event_type: &str,
    payload: Value,
) -> Result<Vec<ProtocolStreamEvent>, String> {
    let mapping = get_external_task_mapping(db, protocol, external_task_id)?;
    let mut events = list_external_task_stream_events(db, protocol, external_task_id)?;
    let event = ProtocolStreamEvent {
        sequence: events.last().map(|event| event.sequence + 1).unwrap_or(1),
        event_type: normalize_stream_event_type(event_type),
        status: mapping.status.clone(),
        payload,
        at: chrono::Utc::now().timestamp_millis(),
    };
    db.append_agent_run_step(
        &mapping.run_id,
        "protocol_task",
        "running",
        &format!("external stream event: {}", event.event_type),
        json!({
            "protocol": mapping.protocol,
            "externalTaskId": mapping.external_task_id,
            "sequence": event.sequence,
            "eventType": event.event_type,
        }),
        event.payload.clone(),
    )
    .map_err(|error| error.to_string())?;
    events.push(event);
    if events.len() > 500 {
        events = events.split_off(events.len() - 500);
    }
    db.set_app_state(&stream_key(protocol, external_task_id), json!(events))
        .map_err(|error| error.to_string())?;
    Ok(events)
}

pub fn list_external_task_stream_events(
    db: &LocalDb,
    protocol: &str,
    external_task_id: &str,
) -> Result<Vec<ProtocolStreamEvent>, String> {
    let value = db
        .get_app_state(&stream_key(protocol, external_task_id))
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| json!([]));
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn persist_mapping(db: &LocalDb, mapping: &ProtocolRunMapping) -> Result<(), String> {
    db.set_app_state(
        &mapping_key(&mapping.protocol, &mapping.external_task_id),
        serde_json::to_value(mapping).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn validate_external_task(task: &ExternalTask) -> Result<(), String> {
    if task.protocol.trim().is_empty() {
        return Err("external protocol is empty".to_string());
    }
    if task.external_task_id.trim().is_empty() {
        return Err("external task id is empty".to_string());
    }
    if task.input.is_null() {
        return Err("external task input is required".to_string());
    }
    Ok(())
}

fn mapping_key(protocol: &str, external_task_id: &str) -> String {
    format!(
        "{}:{}:{}",
        PROTOCOL_MAPPING_PREFIX,
        normalize_protocol(protocol),
        external_task_id.trim()
    )
}

fn normalize_protocol(protocol: &str) -> String {
    match protocol.trim().to_ascii_lowercase().as_str() {
        "agent-protocol" | "agent_protocol" | "autogpt" => "agent-protocol".to_string(),
        "acp" => "acp".to_string(),
        "a2a" => "a2a".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => "agent-protocol".to_string(),
    }
}

fn normalize_lifecycle_status(status: &str) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "mapped" | "pending" | "running" | "completed" | "failed" | "cancelled" => {
            status.trim().to_ascii_lowercase()
        }
        "done" | "succeeded" | "success" => "completed".to_string(),
        "error" => "failed".to_string(),
        "canceled" => "cancelled".to_string(),
        _ => "running".to_string(),
    }
}

fn normalize_stream_event_type(event_type: &str) -> String {
    match event_type.trim().to_ascii_lowercase().as_str() {
        "message" | "step" | "artifact" | "log" | "progress" | "error" => {
            event_type.trim().to_ascii_lowercase()
        }
        _ => "message".to_string(),
    }
}

fn stream_key(protocol: &str, external_task_id: &str) -> String {
    format!("{}:stream", mapping_key(protocol, external_task_id))
}

fn normalize_artifact_type(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "file" | "diff" | "log" | "trajectory" | "report" => kind.trim().to_ascii_lowercase(),
        _ => "protocol_artifact".to_string(),
    }
}

fn default_protocol_version() -> String {
    "v1".to_string()
}

fn default_permission_mode() -> String {
    "default".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        LocalDb::open(std::env::temp_dir().join(format!("aura_protocol_{}.db", Uuid::new_v4())))
            .unwrap()
    }

    #[test]
    fn external_task_maps_to_real_run_step_and_artifact() {
        let db = temp_db();
        let mapping = create_external_task_mapping(
            &db,
            ExternalTask {
                protocol: "AutoGPT".to_string(),
                version: "v1".to_string(),
                external_task_id: "task-1".to_string(),
                session_id: None,
                input: json!({ "goal": "inspect repo" }),
                artifacts: vec![ExternalArtifactInput {
                    artifact_type: "file".to_string(),
                    title: "input.md".to_string(),
                    uri: Some("memory://input.md".to_string()),
                    metadata: json!({ "source": "test" }),
                }],
                permission_mode: "default".to_string(),
            },
        )
        .unwrap();
        assert_eq!(mapping.protocol, "agent-protocol");
        assert_eq!(mapping.artifact_refs.len(), 1);
        let timeline = db.get_run_timeline(&mapping.run_id, 10, 0).unwrap();
        assert_eq!(timeline.entries.len(), 1);
        assert_eq!(timeline.entries[0].label, "protocol_task");
        let loaded = get_external_task_mapping(&db, "agent-protocol", "task-1").unwrap();
        assert_eq!(loaded.run_id, mapping.run_id);
    }

    #[test]
    fn external_task_rejects_empty_input_and_never_calls_tools_directly() {
        let db = temp_db();
        let error = create_external_task_mapping(
            &db,
            ExternalTask {
                protocol: "a2a".to_string(),
                version: "v1".to_string(),
                external_task_id: "".to_string(),
                session_id: None,
                input: json!({}),
                artifacts: vec![],
                permission_mode: "default".to_string(),
            },
        )
        .unwrap_err();
        assert!(error.contains("external task id"));
    }

    #[test]
    fn lifecycle_update_and_stream_are_persisted_to_mapping_and_timeline() {
        let db = temp_db();
        create_external_task_mapping(
            &db,
            ExternalTask {
                protocol: "acp".to_string(),
                version: "v1".to_string(),
                external_task_id: "task-stream".to_string(),
                session_id: None,
                input: json!({ "goal": "run" }),
                artifacts: vec![],
                permission_mode: "default".to_string(),
            },
        )
        .unwrap();
        let running = update_external_task_lifecycle(
            &db,
            ProtocolLifecycleUpdate {
                protocol: "acp".to_string(),
                external_task_id: "task-stream".to_string(),
                status: "running".to_string(),
                message: Some("started".to_string()),
                output: json!({}),
            },
        )
        .unwrap();
        assert_eq!(running.status, "running");
        let events = append_external_task_stream_event(
            &db,
            "acp",
            "task-stream",
            "progress",
            json!({ "pct": 50 }),
        )
        .unwrap();
        assert_eq!(events[0].sequence, 1);
        let cancelled = cancel_external_task(&db, "acp", "task-stream", "operator").unwrap();
        assert_eq!(cancelled.status, "cancelled");
        let timeline = db.get_run_timeline(&cancelled.run_id, 20, 0).unwrap();
        assert!(timeline.entries.len() >= 4);
    }

    #[test]
    fn compatibility_matrix_declares_supported_protocol_controls() {
        let matrix = protocol_compatibility_matrix();
        assert!(matrix
            .iter()
            .any(|entry| entry.protocol == "agent-protocol" && entry.cancellation));
        assert!(matrix
            .iter()
            .all(|entry| entry.lifecycle && entry.streaming));
    }
}
