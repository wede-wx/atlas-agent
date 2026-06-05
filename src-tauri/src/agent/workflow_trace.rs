use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::agent::graph_runtime::{AgentGraphNodeExecutor, DurableAgentGraphRuntime};
use crate::storage::{AgentGraphNodeRecord, AgentGraphSnapshot, LocalDb};
use crate::tools::secret_scan::{scan, SecretAction, SecretLocation};

const GRAPH_QUEUE_KEY: &str = "agent_graph_queue:v1";
const GRAPH_QUEUE_CONTROL_KEY: &str = "agent_graph_queue_control:v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeExecutionTrace {
    pub node_id: String,
    pub node_key: String,
    pub kind: String,
    pub status: String,
    pub attempt: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub input: Value,
    pub output: Value,
    pub error: Option<String>,
    pub redaction_state: String,
    pub findings_redacted: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTraceReport {
    pub graph_run_id: String,
    pub node_count: usize,
    pub redaction_state: String,
    pub findings_redacted: usize,
    pub traces: Vec<NodeExecutionTrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueuedGraphRun {
    pub id: String,
    pub graph_run_id: String,
    pub status: String,
    pub priority: i64,
    pub reason: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowQueueControl {
    pub paused: bool,
    pub reason: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowWorkerPolicy {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub paused: bool,
    #[serde(default = "default_credential_policy")]
    pub credential_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowQueueWorkerReport {
    pub queue_id: Option<String>,
    pub graph_run_id: Option<String>,
    pub status: String,
    pub reason: String,
    pub executed_node_count: usize,
    pub queue: Vec<QueuedGraphRun>,
}

pub fn graph_node_traces(db: &LocalDb, graph_run_id: &str) -> Result<WorkflowTraceReport, String> {
    let snapshot = db
        .get_agent_graph_snapshot(graph_run_id)
        .map_err(|error| error.to_string())?;
    Ok(trace_from_snapshot(&snapshot, true))
}

pub fn trace_from_snapshot(snapshot: &AgentGraphSnapshot, redact: bool) -> WorkflowTraceReport {
    let mut findings = 0usize;
    let traces = snapshot
        .nodes
        .iter()
        .map(|node| {
            let (input, input_findings) = redact_value(node.input.clone(), redact);
            let (output, output_findings) = redact_value(node.output.clone(), redact);
            findings += input_findings + output_findings;
            node_trace(
                node,
                input,
                output,
                input_findings + output_findings,
                redact,
            )
        })
        .collect::<Vec<_>>();
    WorkflowTraceReport {
        graph_run_id: snapshot.run.id.clone(),
        node_count: traces.len(),
        redaction_state: if redact { "masked" } else { "raw" }.to_string(),
        findings_redacted: findings,
        traces,
    }
}

pub fn enqueue_graph_run(
    db: &LocalDb,
    graph_run_id: &str,
    priority: Option<i64>,
) -> Result<QueuedGraphRun, String> {
    db.get_agent_graph_snapshot(graph_run_id)
        .map_err(|error| error.to_string())?;
    let now = chrono::Utc::now().timestamp_millis();
    let queued = QueuedGraphRun {
        id: format!("gq_{}", Uuid::new_v4()),
        graph_run_id: graph_run_id.to_string(),
        status: "queued".to_string(),
        priority: priority.unwrap_or(0).clamp(-100, 100),
        reason: "queued through workflow trace architecture".to_string(),
        created_at: now,
        updated_at: now,
    };
    let mut queue = list_graph_queue(db)?;
    queue.push(queued.clone());
    persist_graph_queue(db, &queue)?;
    Ok(queued)
}

pub fn set_graph_queue_paused(
    db: &LocalDb,
    paused: bool,
    reason: &str,
) -> Result<WorkflowQueueControl, String> {
    let control = WorkflowQueueControl {
        paused,
        reason: reason.chars().take(300).collect(),
        updated_at: chrono::Utc::now().timestamp_millis(),
    };
    db.set_app_state(GRAPH_QUEUE_CONTROL_KEY, json!(control))
        .map_err(|error| error.to_string())?;
    Ok(control)
}

pub fn get_graph_queue_control(db: &LocalDb) -> Result<WorkflowQueueControl, String> {
    let value = db
        .get_app_state(GRAPH_QUEUE_CONTROL_KEY)
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| {
            json!(WorkflowQueueControl {
                paused: false,
                reason: "queue active".to_string(),
                updated_at: chrono::Utc::now().timestamp_millis(),
            })
        });
    serde_json::from_value(value).map_err(|error| error.to_string())
}

pub fn run_next_queued_graph_with_executor<E: AgentGraphNodeExecutor>(
    db: &LocalDb,
    policy: WorkflowWorkerPolicy,
    executor: &mut E,
) -> Result<WorkflowQueueWorkerReport, String> {
    let control = get_graph_queue_control(db)?;
    let mut queue = list_graph_queue(db)?;
    if policy.paused || control.paused {
        return Ok(WorkflowQueueWorkerReport {
            queue_id: None,
            graph_run_id: None,
            status: "paused".to_string(),
            reason: if policy.paused {
                "worker policy paused".to_string()
            } else {
                control.reason
            },
            executed_node_count: 0,
            queue,
        });
    }
    let Some(index) = next_queue_index(&queue) else {
        return Ok(WorkflowQueueWorkerReport {
            queue_id: None,
            graph_run_id: None,
            status: "idle".to_string(),
            reason: "no queued graph run".to_string(),
            executed_node_count: 0,
            queue,
        });
    };
    let queued = queue[index].clone();
    let snapshot = db
        .get_agent_graph_snapshot(&queued.graph_run_id)
        .map_err(|error| error.to_string())?;
    if let Some(reason) = credential_policy_block_reason(&snapshot, &policy.credential_policy) {
        update_queue_item(&mut queue, index, "blocked", &reason);
        persist_graph_queue(db, &queue)?;
        return Ok(WorkflowQueueWorkerReport {
            queue_id: Some(queued.id),
            graph_run_id: Some(queued.graph_run_id),
            status: "blocked".to_string(),
            reason,
            executed_node_count: 0,
            queue,
        });
    }
    if policy.dry_run {
        update_queue_item(
            &mut queue,
            index,
            "planned",
            "dry-run worker did not execute nodes",
        );
        persist_graph_queue(db, &queue)?;
        return Ok(WorkflowQueueWorkerReport {
            queue_id: Some(queued.id),
            graph_run_id: Some(queued.graph_run_id),
            status: "planned".to_string(),
            reason: "dry-run worker did not execute nodes".to_string(),
            executed_node_count: 0,
            queue,
        });
    }
    update_queue_item(&mut queue, index, "running", "worker started graph run");
    persist_graph_queue(db, &queue)?;
    let before = snapshot
        .nodes
        .iter()
        .filter(|node| matches!(node.status.as_str(), "succeeded" | "failed" | "skipped"))
        .count();
    let runtime = DurableAgentGraphRuntime::new(db.clone());
    let finished = runtime
        .run_until_blocked_or_finished(&queued.graph_run_id, executor)
        .map_err(|error| error.to_string())?;
    let after = finished
        .nodes
        .iter()
        .filter(|node| matches!(node.status.as_str(), "succeeded" | "failed" | "skipped"))
        .count();
    let mut queue = list_graph_queue(db)?;
    if let Some(current_index) = queue.iter().position(|item| item.id == queued.id) {
        update_queue_item(
            &mut queue,
            current_index,
            &finished.run.status,
            &format!("graph run ended with {}", finished.run.status),
        );
        persist_graph_queue(db, &queue)?;
    }
    Ok(WorkflowQueueWorkerReport {
        queue_id: Some(queued.id),
        graph_run_id: Some(queued.graph_run_id),
        status: finished.run.status.clone(),
        reason: format!("graph run ended with {}", finished.run.status),
        executed_node_count: after.saturating_sub(before),
        queue,
    })
}

pub fn abort_queued_graph_run(
    db: &LocalDb,
    queue_id: &str,
    reason: &str,
) -> Result<QueuedGraphRun, String> {
    let mut queue = list_graph_queue(db)?;
    let now = chrono::Utc::now().timestamp_millis();
    let mut updated = None;
    for item in &mut queue {
        if item.id == queue_id {
            item.status = "aborted".to_string();
            item.reason = reason.chars().take(300).collect();
            item.updated_at = now;
            updated = Some(item.clone());
            break;
        }
    }
    let updated = updated.ok_or_else(|| format!("queued graph run not found: {queue_id}"))?;
    persist_graph_queue(db, &queue)?;
    Ok(updated)
}

pub fn list_graph_queue(db: &LocalDb) -> Result<Vec<QueuedGraphRun>, String> {
    let value = db
        .get_app_state(GRAPH_QUEUE_KEY)
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| json!([]));
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn persist_graph_queue(db: &LocalDb, queue: &[QueuedGraphRun]) -> Result<(), String> {
    db.set_app_state(GRAPH_QUEUE_KEY, json!(queue))
        .map_err(|error| error.to_string())
}

fn next_queue_index(queue: &[QueuedGraphRun]) -> Option<usize> {
    queue
        .iter()
        .enumerate()
        .filter(|(_, item)| item.status == "queued")
        .max_by_key(|(_, item)| (item.priority, std::cmp::Reverse(item.created_at)))
        .map(|(index, _)| index)
}

fn update_queue_item(queue: &mut [QueuedGraphRun], index: usize, status: &str, reason: &str) {
    if let Some(item) = queue.get_mut(index) {
        item.status = status.to_string();
        item.reason = reason.chars().take(300).collect();
        item.updated_at = chrono::Utc::now().timestamp_millis();
    }
}

fn credential_policy_block_reason(
    snapshot: &AgentGraphSnapshot,
    credential_policy: &str,
) -> Option<String> {
    let policy = credential_policy.trim();
    if policy == "allow" {
        return None;
    }
    for node in &snapshot.nodes {
        let text = node.input.to_string().to_ascii_lowercase();
        let has_secret_material = text.contains("apikey")
            || text.contains("api_key")
            || text.contains("token")
            || text.contains("secret")
            || text.contains("password");
        if policy == "deny_all" && has_secret_material {
            return Some(format!(
                "credential policy deny_all blocked node {}",
                node.node_key
            ));
        }
        if policy == "deny_network"
            && has_secret_material
            && matches!(node.kind.as_str(), "browser" | "tool" | "agent")
        {
            return Some(format!(
                "credential policy deny_network blocked node {}",
                node.node_key
            ));
        }
    }
    None
}

fn default_credential_policy() -> String {
    "allow".to_string()
}

fn node_trace(
    node: &AgentGraphNodeRecord,
    input: Value,
    output: Value,
    findings_redacted: usize,
    redact: bool,
) -> NodeExecutionTrace {
    NodeExecutionTrace {
        node_id: node.id.clone(),
        node_key: node.node_key.clone(),
        kind: node.kind.clone(),
        status: node.status.clone(),
        attempt: node.attempt,
        started_at: node.started_at,
        finished_at: node.finished_at,
        input,
        output,
        error: node.error.clone(),
        redaction_state: if redact { "masked" } else { "raw" }.to_string(),
        findings_redacted,
    }
}

fn redact_value(value: Value, enabled: bool) -> (Value, usize) {
    if !enabled {
        return (value, 0);
    }
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    let report = scan(&serialized, SecretLocation::Log, SecretAction::Masked);
    let redacted = serde_json::from_str(&report.text).unwrap_or(Value::String(report.text));
    (redacted, report.findings.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateAgentGraphNodePayload, CreateAgentGraphRunPayload, LocalDb};
    use serde_json::json;
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        LocalDb::open(
            std::env::temp_dir().join(format!("atlas_workflow_trace_{}.db", Uuid::new_v4())),
        )
        .unwrap()
    }

    #[test]
    fn graph_trace_redacts_credentials_from_node_payloads() {
        let db = temp_db();
        let run = db
            .create_agent_graph_run(CreateAgentGraphRunPayload {
                id: Some("graph-a".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "trace".to_string(),
            })
            .unwrap();
        db.create_agent_graph_node(CreateAgentGraphNodePayload {
            id: Some("node-a".to_string()),
            graph_run_id: run.id.clone(),
            node_key: "agent".to_string(),
            kind: "agent".to_string(),
            title: "Agent".to_string(),
            max_attempts: Some(1),
            input: json!({ "apiKey": format!("{}{}", "sk", "-proj-abcdefghijklmnopqrstuvwxyz") }),
        })
        .unwrap();
        let report = graph_node_traces(&db, "graph-a").unwrap();
        assert_eq!(report.node_count, 1);
        assert!(!report.traces[0].input.to_string().contains("sk-proj-"));
    }

    #[test]
    fn graph_queue_can_abort_without_running_nodes() {
        let db = temp_db();
        db.create_agent_graph_run(CreateAgentGraphRunPayload {
            id: Some("graph-q".to_string()),
            session_id: None,
            source_run_id: None,
            goal: "queued".to_string(),
        })
        .unwrap();
        let queued = enqueue_graph_run(&db, "graph-q", Some(5)).unwrap();
        let aborted = abort_queued_graph_run(&db, &queued.id, "user cancelled").unwrap();
        assert_eq!(aborted.status, "aborted");
    }

    #[test]
    fn queue_worker_runs_next_graph_with_real_runtime() {
        let db = temp_db();
        let runtime = DurableAgentGraphRuntime::new(db.clone());
        runtime
            .create_run(crate::agent::graph_runtime::CreateAgentGraphSpec {
                id: Some("graph-worker".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "worker".to_string(),
                nodes: vec![crate::agent::graph_runtime::AgentGraphNodeSpec {
                    node_key: "agent".to_string(),
                    kind: "agent".to_string(),
                    title: "Agent".to_string(),
                    max_attempts: Some(1),
                    input: json!({}),
                }],
                edges: vec![],
            })
            .unwrap();
        enqueue_graph_run(&db, "graph-worker", Some(1)).unwrap();
        let report = run_next_queued_graph_with_executor(
            &db,
            WorkflowWorkerPolicy {
                dry_run: false,
                paused: false,
                credential_policy: "allow".to_string(),
            },
            &mut |node: &AgentGraphNodeRecord| Ok(json!({ "node": node.node_key })),
        )
        .unwrap();
        assert_eq!(report.status, "succeeded");
        assert_eq!(report.executed_node_count, 1);
        assert_eq!(list_graph_queue(&db).unwrap()[0].status, "succeeded");
    }

    #[test]
    fn queue_pause_and_credential_policy_block_execution() {
        let db = temp_db();
        let runtime = DurableAgentGraphRuntime::new(db.clone());
        runtime
            .create_run(crate::agent::graph_runtime::CreateAgentGraphSpec {
                id: Some("graph-secret".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "secret".to_string(),
                nodes: vec![crate::agent::graph_runtime::AgentGraphNodeSpec {
                    node_key: "tool".to_string(),
                    kind: "tool".to_string(),
                    title: "Tool".to_string(),
                    max_attempts: Some(1),
                    input: json!({ "apiKey": "secret" }),
                }],
                edges: vec![],
            })
            .unwrap();
        enqueue_graph_run(&db, "graph-secret", Some(1)).unwrap();
        set_graph_queue_paused(&db, true, "operator pause").unwrap();
        let paused = run_next_queued_graph_with_executor(
            &db,
            WorkflowWorkerPolicy {
                dry_run: false,
                paused: false,
                credential_policy: "allow".to_string(),
            },
            &mut |_: &AgentGraphNodeRecord| Ok(json!({})),
        )
        .unwrap();
        assert_eq!(paused.status, "paused");
        set_graph_queue_paused(&db, false, "resume").unwrap();
        let blocked = run_next_queued_graph_with_executor(
            &db,
            WorkflowWorkerPolicy {
                dry_run: false,
                paused: false,
                credential_policy: "deny_all".to_string(),
            },
            &mut |_: &AgentGraphNodeRecord| Ok(json!({})),
        )
        .unwrap();
        assert_eq!(blocked.status, "blocked");
        assert_eq!(list_graph_queue(&db).unwrap()[0].status, "blocked");
    }
}
