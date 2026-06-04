use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::storage::{
    AgentGraphNodeRecord, AgentGraphSnapshot, CreateAgentGraphEdgePayload,
    CreateAgentGraphNodePayload, CreateAgentGraphRunPayload, LocalDb, StorageError, StorageResult,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphNodeSpec {
    pub node_key: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub max_attempts: Option<i64>,
    #[serde(default)]
    pub input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphEdgeSpec {
    pub from_node_key: String,
    pub to_node_key: String,
    #[serde(default)]
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentGraphSpec {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub source_run_id: Option<String>,
    pub goal: String,
    #[serde(default)]
    pub nodes: Vec<AgentGraphNodeSpec>,
    #[serde(default)]
    pub edges: Vec<AgentGraphEdgeSpec>,
}

pub trait AgentGraphNodeExecutor {
    fn execute_node(&mut self, node: &AgentGraphNodeRecord) -> Result<Value, String>;
}

pub trait AgentGraphKindExecutor {
    fn execute_agent_node(&mut self, node: &AgentGraphNodeRecord) -> Result<Value, String>;
    fn execute_tool_node(&mut self, node: &AgentGraphNodeRecord) -> Result<Value, String>;
    fn execute_verifier_node(&mut self, node: &AgentGraphNodeRecord) -> Result<Value, String>;
}

pub struct AgentGraphKindDispatcher<'a, E: AgentGraphKindExecutor> {
    executor: &'a mut E,
}

impl<E: AgentGraphKindExecutor> AgentGraphNodeExecutor for AgentGraphKindDispatcher<'_, E> {
    fn execute_node(&mut self, node: &AgentGraphNodeRecord) -> Result<Value, String> {
        match node.kind.as_str() {
            "agent" => self.executor.execute_agent_node(node),
            "tool" => self.executor.execute_tool_node(node),
            "verifier" => self.executor.execute_verifier_node(node),
            other => Err(format!(
                "no graph executor registered for node kind: {other}"
            )),
        }
    }
}

impl<F> AgentGraphNodeExecutor for F
where
    F: FnMut(&AgentGraphNodeRecord) -> Result<Value, String>,
{
    fn execute_node(&mut self, node: &AgentGraphNodeRecord) -> Result<Value, String> {
        self(node)
    }
}

#[derive(Debug, Clone)]
pub struct DurableAgentGraphRuntime {
    db: LocalDb,
}

impl DurableAgentGraphRuntime {
    pub fn new(db: LocalDb) -> Self {
        Self { db }
    }

    pub fn create_run(&self, spec: CreateAgentGraphSpec) -> StorageResult<AgentGraphSnapshot> {
        validate_graph_spec(&spec)?;
        let run = self.db.create_agent_graph_run(CreateAgentGraphRunPayload {
            id: spec.id,
            session_id: spec.session_id,
            source_run_id: spec.source_run_id,
            goal: spec.goal,
        })?;

        let mut node_ids = BTreeMap::new();
        for node in spec.nodes {
            let record = self
                .db
                .create_agent_graph_node(CreateAgentGraphNodePayload {
                    id: None,
                    graph_run_id: run.id.clone(),
                    node_key: node.node_key.clone(),
                    kind: node.kind,
                    title: node.title,
                    max_attempts: node.max_attempts,
                    input: node.input,
                })?;
            node_ids.insert(node.node_key, record.id);
        }

        for edge in spec.edges {
            let from_node_id = node_ids
                .get(&edge.from_node_key)
                .cloned()
                .ok_or_else(|| StorageError::Validation("unknown graph edge source".to_string()))?;
            let to_node_id = node_ids
                .get(&edge.to_node_key)
                .cloned()
                .ok_or_else(|| StorageError::Validation("unknown graph edge target".to_string()))?;
            self.db
                .create_agent_graph_edge(CreateAgentGraphEdgePayload {
                    id: None,
                    graph_run_id: run.id.clone(),
                    from_node_id,
                    to_node_id,
                    condition: edge.condition,
                })?;
        }

        self.db.get_agent_graph_snapshot(&run.id)
    }

    pub fn run_until_blocked_or_finished<E: AgentGraphNodeExecutor>(
        &self,
        graph_run_id: &str,
        executor: &mut E,
    ) -> StorageResult<AgentGraphSnapshot> {
        let mut snapshot = self.db.get_agent_graph_snapshot(graph_run_id)?;
        if is_graph_terminal(&snapshot.run.status) || snapshot.run.status == "paused" {
            return Ok(snapshot);
        }
        self.db
            .update_agent_graph_run_status(graph_run_id, "running", None)?;

        let mut remaining_steps = snapshot.nodes.len().saturating_mul(4).saturating_add(10);
        loop {
            if remaining_steps == 0 {
                self.db.update_agent_graph_run_status(
                    graph_run_id,
                    "blocked",
                    Some("agent graph loop budget exhausted"),
                )?;
                return self.db.get_agent_graph_snapshot(graph_run_id);
            }
            remaining_steps -= 1;

            snapshot = self.db.get_agent_graph_snapshot(graph_run_id)?;
            if is_graph_terminal(&snapshot.run.status) {
                return Ok(snapshot);
            }
            if snapshot.run.status == "paused" {
                return Ok(snapshot);
            }

            let ready = ready_nodes(&snapshot);
            if ready.is_empty() {
                let final_status = graph_completion_status(&snapshot);
                self.db.update_agent_graph_run_status(
                    graph_run_id,
                    &final_status,
                    (final_status == "blocked")
                        .then_some("no runnable graph nodes and graph is not terminal"),
                )?;
                return self.db.get_agent_graph_snapshot(graph_run_id);
            }

            for node in ready {
                let started = self.db.start_agent_graph_node(&node.id)?;
                match executor.execute_node(&started) {
                    Ok(output) => {
                        let finished = self.db.finish_agent_graph_node(
                            &started.id,
                            "succeeded",
                            output.clone(),
                            None,
                        )?;
                        self.db.record_agent_graph_checkpoint(
                            graph_run_id,
                            Some(&finished.id),
                            json!({
                                "nodeId": finished.id,
                                "nodeKey": finished.node_key,
                                "status": finished.status,
                                "attempt": finished.attempt,
                                "output": output,
                            }),
                        )?;
                    }
                    Err(error) => {
                        if started.attempt < started.max_attempts {
                            self.db.finish_agent_graph_node(
                                &started.id,
                                "pending",
                                json!({}),
                                Some(&error),
                            )?;
                        } else {
                            self.db.finish_agent_graph_node(
                                &started.id,
                                "failed",
                                json!({}),
                                Some(&error),
                            )?;
                            self.db.update_agent_graph_run_status(
                                graph_run_id,
                                "failed",
                                Some(&error),
                            )?;
                            return self.db.get_agent_graph_snapshot(graph_run_id);
                        }
                    }
                }
            }
        }
    }

    pub fn run_with_kind_executor<E: AgentGraphKindExecutor>(
        &self,
        graph_run_id: &str,
        executor: &mut E,
    ) -> StorageResult<AgentGraphSnapshot> {
        let mut dispatcher = AgentGraphKindDispatcher { executor };
        self.run_until_blocked_or_finished(graph_run_id, &mut dispatcher)
    }

    pub fn pause_run(&self, graph_run_id: &str, reason: &str) -> StorageResult<AgentGraphSnapshot> {
        self.db
            .update_agent_graph_run_status(graph_run_id, "paused", Some(reason))?;
        self.db.get_agent_graph_snapshot(graph_run_id)
    }

    pub fn resume_run(
        &self,
        graph_run_id: &str,
        reason: &str,
    ) -> StorageResult<AgentGraphSnapshot> {
        let snapshot = self.db.get_agent_graph_snapshot(graph_run_id)?;
        if is_graph_terminal(&snapshot.run.status) {
            return Err(StorageError::Validation(
                "terminal graph run cannot be resumed".to_string(),
            ));
        }
        self.db
            .update_agent_graph_run_status(graph_run_id, "running", Some(reason))?;
        self.db.get_agent_graph_snapshot(graph_run_id)
    }
}

fn validate_graph_spec(spec: &CreateAgentGraphSpec) -> StorageResult<()> {
    if spec.goal.trim().is_empty() {
        return Err(StorageError::Validation(
            "agent graph goal is empty".to_string(),
        ));
    }
    if spec.nodes.is_empty() {
        return Err(StorageError::Validation(
            "agent graph requires at least one node".to_string(),
        ));
    }
    let mut keys = BTreeSet::new();
    for node in &spec.nodes {
        let key = node.node_key.trim();
        if key.is_empty() {
            return Err(StorageError::Validation(
                "agent graph node key is empty".to_string(),
            ));
        }
        if !keys.insert(key.to_string()) {
            return Err(StorageError::Validation(format!(
                "duplicate agent graph node key: {key}"
            )));
        }
    }
    for edge in &spec.edges {
        let from = edge.from_node_key.trim();
        let to = edge.to_node_key.trim();
        if !keys.contains(from) || !keys.contains(to) {
            return Err(StorageError::Validation(format!(
                "agent graph edge references unknown node: {from}->{to}"
            )));
        }
        if from == to {
            return Err(StorageError::Validation(
                "agent graph edge cannot point to the same node".to_string(),
            ));
        }
    }
    if has_cycle(&keys, &spec.edges) {
        return Err(StorageError::Validation(
            "agent graph edges must be acyclic".to_string(),
        ));
    }
    Ok(())
}

fn has_cycle(keys: &BTreeSet<String>, edges: &[AgentGraphEdgeSpec]) -> bool {
    let mut incoming = keys
        .iter()
        .map(|key| (key.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    let mut outgoing = keys
        .iter()
        .map(|key| (key.clone(), Vec::<String>::new()))
        .collect::<BTreeMap<_, _>>();
    for edge in edges {
        if let Some(count) = incoming.get_mut(edge.to_node_key.trim()) {
            *count += 1;
        }
        if let Some(list) = outgoing.get_mut(edge.from_node_key.trim()) {
            list.push(edge.to_node_key.trim().to_string());
        }
    }

    let mut queue = incoming
        .iter()
        .filter_map(|(key, count)| (*count == 0).then_some(key.clone()))
        .collect::<Vec<_>>();
    let mut visited = 0usize;
    while let Some(key) = queue.pop() {
        visited += 1;
        if let Some(children) = outgoing.get(&key) {
            for child in children {
                if let Some(count) = incoming.get_mut(child) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        queue.push(child.clone());
                    }
                }
            }
        }
    }
    visited != keys.len()
}

fn ready_nodes(snapshot: &AgentGraphSnapshot) -> Vec<AgentGraphNodeRecord> {
    let nodes_by_id = snapshot
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    snapshot
        .nodes
        .iter()
        .filter(|node| node.status == "pending")
        .filter(|node| {
            snapshot
                .edges
                .iter()
                .filter(|edge| edge.to_node_id == node.id)
                .all(|edge| {
                    nodes_by_id
                        .get(edge.from_node_id.as_str())
                        .map(|source| source.status == "succeeded" || source.status == "skipped")
                        .unwrap_or(false)
                })
        })
        .cloned()
        .collect()
}

fn graph_completion_status(snapshot: &AgentGraphSnapshot) -> String {
    if snapshot.nodes.iter().any(|node| node.status == "failed") {
        return "failed".to_string();
    }
    if snapshot
        .nodes
        .iter()
        .all(|node| matches!(node.status.as_str(), "succeeded" | "skipped"))
    {
        return "succeeded".to_string();
    }
    "blocked".to_string()
}

fn is_graph_terminal(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        let path = std::env::temp_dir().join(format!("aura_graph_{}.db", Uuid::new_v4()));
        LocalDb::open(path).unwrap()
    }

    fn node(key: &str, kind: &str) -> AgentGraphNodeSpec {
        AgentGraphNodeSpec {
            node_key: key.to_string(),
            kind: kind.to_string(),
            title: key.to_string(),
            max_attempts: Some(1),
            input: json!({ "key": key }),
        }
    }

    struct KindExecutor {
        calls: Vec<String>,
    }

    impl AgentGraphKindExecutor for KindExecutor {
        fn execute_agent_node(&mut self, node: &AgentGraphNodeRecord) -> Result<Value, String> {
            self.calls.push(format!("agent:{}", node.node_key));
            Ok(json!({ "agent": node.node_key }))
        }

        fn execute_tool_node(&mut self, node: &AgentGraphNodeRecord) -> Result<Value, String> {
            self.calls.push(format!("tool:{}", node.node_key));
            Ok(json!({ "tool": node.node_key }))
        }

        fn execute_verifier_node(&mut self, node: &AgentGraphNodeRecord) -> Result<Value, String> {
            self.calls.push(format!("verifier:{}", node.node_key));
            Ok(json!({ "verifier": node.node_key }))
        }
    }

    #[test]
    fn durable_graph_runs_nodes_in_dependency_order() {
        let runtime = DurableAgentGraphRuntime::new(temp_db());
        let snapshot = runtime
            .create_run(CreateAgentGraphSpec {
                id: Some("graph-a".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "test graph".to_string(),
                nodes: vec![
                    node("agent", "agent"),
                    node("tool", "tool"),
                    node("verify", "verifier"),
                ],
                edges: vec![
                    AgentGraphEdgeSpec {
                        from_node_key: "agent".to_string(),
                        to_node_key: "tool".to_string(),
                        condition: None,
                    },
                    AgentGraphEdgeSpec {
                        from_node_key: "tool".to_string(),
                        to_node_key: "verify".to_string(),
                        condition: None,
                    },
                ],
            })
            .unwrap();
        assert_eq!(snapshot.nodes.len(), 3);

        let mut order = Vec::new();
        let done = runtime
            .run_until_blocked_or_finished("graph-a", &mut |node: &AgentGraphNodeRecord| {
                order.push(node.node_key.clone());
                Ok(json!({ "done": node.node_key }))
            })
            .unwrap();

        assert_eq!(done.run.status, "succeeded");
        assert_eq!(order, ["agent", "tool", "verify"]);
        assert_eq!(done.checkpoints.len(), 3);
    }

    #[test]
    fn durable_graph_retries_until_max_attempts() {
        let runtime = DurableAgentGraphRuntime::new(temp_db());
        runtime
            .create_run(CreateAgentGraphSpec {
                id: Some("graph-retry".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "retry".to_string(),
                nodes: vec![AgentGraphNodeSpec {
                    max_attempts: Some(2),
                    ..node("tool", "tool")
                }],
                edges: vec![],
            })
            .unwrap();

        let mut calls = 0;
        let done = runtime
            .run_until_blocked_or_finished("graph-retry", &mut |_: &AgentGraphNodeRecord| {
                calls += 1;
                if calls == 1 {
                    Err("temporary".to_string())
                } else {
                    Ok(json!({ "ok": true }))
                }
            })
            .unwrap();

        assert_eq!(done.run.status, "succeeded");
        assert_eq!(done.nodes[0].attempt, 2);
        assert_eq!(done.nodes[0].status, "succeeded");
    }

    #[test]
    fn durable_graph_rejects_cycles_before_writing() {
        let runtime = DurableAgentGraphRuntime::new(temp_db());
        let result = runtime.create_run(CreateAgentGraphSpec {
            id: Some("graph-cycle".to_string()),
            session_id: None,
            source_run_id: None,
            goal: "cycle".to_string(),
            nodes: vec![node("a", "agent"), node("b", "tool")],
            edges: vec![
                AgentGraphEdgeSpec {
                    from_node_key: "a".to_string(),
                    to_node_key: "b".to_string(),
                    condition: None,
                },
                AgentGraphEdgeSpec {
                    from_node_key: "b".to_string(),
                    to_node_key: "a".to_string(),
                    condition: None,
                },
            ],
        });
        assert!(matches!(result, Err(StorageError::Validation(_))));
    }

    #[test]
    fn graph_kind_executor_dispatches_known_node_kinds() {
        let runtime = DurableAgentGraphRuntime::new(temp_db());
        runtime
            .create_run(CreateAgentGraphSpec {
                id: Some("graph-kind".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "kind dispatch".to_string(),
                nodes: vec![node("a", "agent"), node("b", "tool"), node("c", "verifier")],
                edges: vec![
                    AgentGraphEdgeSpec {
                        from_node_key: "a".to_string(),
                        to_node_key: "b".to_string(),
                        condition: None,
                    },
                    AgentGraphEdgeSpec {
                        from_node_key: "b".to_string(),
                        to_node_key: "c".to_string(),
                        condition: None,
                    },
                ],
            })
            .unwrap();
        let mut executor = KindExecutor { calls: Vec::new() };
        let done = runtime
            .run_with_kind_executor("graph-kind", &mut executor)
            .unwrap();
        assert_eq!(done.run.status, "succeeded");
        assert_eq!(executor.calls, ["agent:a", "tool:b", "verifier:c"]);
    }

    #[test]
    fn graph_pause_prevents_execution_until_resumed() {
        let runtime = DurableAgentGraphRuntime::new(temp_db());
        runtime
            .create_run(CreateAgentGraphSpec {
                id: Some("graph-pause".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "pause".to_string(),
                nodes: vec![node("a", "agent")],
                edges: vec![],
            })
            .unwrap();
        let paused = runtime.pause_run("graph-pause", "operator pause").unwrap();
        assert_eq!(paused.run.status, "paused");
        let mut called = false;
        let after_noop = runtime
            .run_until_blocked_or_finished("graph-pause", &mut |_: &AgentGraphNodeRecord| {
                called = true;
                Ok(json!({}))
            })
            .unwrap();
        assert_eq!(after_noop.run.status, "paused");
        assert!(!called);
        runtime
            .resume_run("graph-pause", "operator resume")
            .unwrap();
        let done = runtime
            .run_until_blocked_or_finished("graph-pause", &mut |_: &AgentGraphNodeRecord| {
                called = true;
                Ok(json!({}))
            })
            .unwrap();
        assert_eq!(done.run.status, "succeeded");
        assert!(called);
    }
}
