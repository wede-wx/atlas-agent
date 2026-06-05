use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::storage::{LocalDb, RunTimelineEntry};
use crate::tools::secret_scan::{scan, SecretAction, SecretLocation};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryExportOptions {
    pub run_id: String,
    #[serde(default = "default_true")]
    pub include_payloads: bool,
    #[serde(default = "default_true")]
    pub redact_secrets: bool,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryEvent {
    pub event_type: String,
    pub run_id: String,
    pub source_id: String,
    pub timestamp: i64,
    pub sequence: i64,
    pub label: String,
    pub status: Option<String>,
    pub redaction_state: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryExport {
    pub run_id: String,
    pub format: String,
    pub event_count: usize,
    pub total_available: i64,
    pub truncated: bool,
    pub redaction_state: String,
    pub findings_redacted: usize,
    pub replay_safe: bool,
    pub exported_at: i64,
    pub events: Vec<TrajectoryEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReplayFrame {
    pub index: usize,
    pub event_type: String,
    pub source_id: String,
    pub timestamp: i64,
    pub summary: String,
    pub would_mutate: bool,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReplayReport {
    pub run_id: String,
    pub frame_count: usize,
    pub external_calls_blocked: bool,
    pub workspace_mutations_blocked: bool,
    pub findings_redacted: usize,
    pub frames: Vec<ReplayFrame>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryImportReport {
    pub format: String,
    pub run_id: Option<String>,
    pub event_count: usize,
    pub truncated: bool,
    pub events: Vec<TrajectoryEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryEvalReport {
    pub run_id: String,
    pub frame_count: usize,
    pub mutating_frame_count: usize,
    pub verification_frame_count: usize,
    pub completion_claim_count: usize,
    pub false_completion_risk: String,
    pub reasons: Vec<String>,
}

pub fn export_run_trajectory(
    db: &LocalDb,
    options: TrajectoryExportOptions,
) -> Result<TrajectoryExport, String> {
    let run_id = options.run_id.trim();
    if run_id.is_empty() {
        return Err("trajectory run_id is empty".to_string());
    }
    let limit = options.limit.unwrap_or(1000).clamp(1, 1000);
    let timeline = db
        .get_run_timeline(run_id, limit, 0)
        .map_err(|error| error.to_string())?;
    let mut findings_redacted = 0usize;
    let events = timeline
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let payload = if options.include_payloads {
                entry.detail.clone()
            } else {
                json!({ "omitted": true })
            };
            let (payload, redacted) = if options.redact_secrets {
                redact_json(payload)
            } else {
                (payload, 0)
            };
            findings_redacted += redacted;
            trajectory_event(run_id, index, entry, payload, options.redact_secrets)
        })
        .collect::<Vec<_>>();
    Ok(TrajectoryExport {
        run_id: run_id.to_string(),
        format: "atlas-trajectory-jsonl-v1".to_string(),
        event_count: events.len(),
        total_available: timeline.total,
        truncated: timeline.total > events.len() as i64,
        redaction_state: if options.redact_secrets {
            "masked".to_string()
        } else {
            "raw".to_string()
        },
        findings_redacted,
        replay_safe: true,
        exported_at: chrono::Utc::now().timestamp_millis(),
        events,
    })
}

pub fn replay_trajectory_readonly(export: &TrajectoryExport) -> ReplayReport {
    let frames = export
        .events
        .iter()
        .enumerate()
        .map(|(index, event)| ReplayFrame {
            index,
            event_type: event.event_type.clone(),
            source_id: event.source_id.clone(),
            timestamp: event.timestamp,
            summary: replay_summary(event),
            would_mutate: event_would_mutate(event),
            payload: event.payload.clone(),
        })
        .collect::<Vec<_>>();
    ReplayReport {
        run_id: export.run_id.clone(),
        frame_count: frames.len(),
        external_calls_blocked: true,
        workspace_mutations_blocked: true,
        findings_redacted: export.findings_redacted,
        frames,
    }
}

pub fn import_trajectory_jsonl(
    text: &str,
    limit: Option<usize>,
) -> Result<TrajectoryImportReport, String> {
    let max_events = limit.unwrap_or(1000).clamp(1, 10_000);
    let mut total = 0usize;
    let mut events = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        total += 1;
        if events.len() >= max_events {
            continue;
        }
        let event = serde_json::from_str::<TrajectoryEvent>(line).map_err(|error| {
            format!("invalid trajectory jsonl line {}: {error}", line_index + 1)
        })?;
        events.push(event);
    }
    let run_id = events.first().map(|event| event.run_id.clone());
    if let Some(run_id) = &run_id {
        if events.iter().any(|event| &event.run_id != run_id) {
            return Err("trajectory jsonl contains multiple run ids".to_string());
        }
    }
    Ok(TrajectoryImportReport {
        format: "atlas-trajectory-jsonl-v1".to_string(),
        run_id,
        event_count: events.len(),
        truncated: total > events.len(),
        events,
    })
}

pub fn evaluate_trajectory_completion(export: &TrajectoryExport) -> TrajectoryEvalReport {
    let replay = replay_trajectory_readonly(export);
    let mutating_frame_count = replay
        .frames
        .iter()
        .filter(|frame| frame.would_mutate)
        .count();
    let verification_frame_count = export
        .events
        .iter()
        .filter(|event| is_verification_event(event))
        .count();
    let completion_claim_count = export
        .events
        .iter()
        .filter(|event| is_completion_claim(event))
        .count();
    let mut reasons = Vec::new();
    if completion_claim_count > 0 && verification_frame_count == 0 {
        reasons.push("completion claim exists without verification evidence".to_string());
    }
    if mutating_frame_count > 0 && verification_frame_count == 0 {
        reasons.push("workspace mutation was replayed without later verification".to_string());
    }
    if export.truncated {
        reasons.push("trajectory export is truncated; eval cannot prove full run".to_string());
    }
    let false_completion_risk = if reasons
        .iter()
        .any(|reason| reason.contains("completion claim"))
    {
        "high"
    } else if reasons.is_empty() {
        "low"
    } else {
        "medium"
    }
    .to_string();
    TrajectoryEvalReport {
        run_id: export.run_id.clone(),
        frame_count: replay.frame_count,
        mutating_frame_count,
        verification_frame_count,
        completion_claim_count,
        false_completion_risk,
        reasons,
    }
}

fn trajectory_event(
    run_id: &str,
    index: usize,
    entry: &RunTimelineEntry,
    payload: Value,
    redacted: bool,
) -> TrajectoryEvent {
    TrajectoryEvent {
        event_type: entry.kind.clone(),
        run_id: run_id.to_string(),
        source_id: entry.id.clone(),
        timestamp: entry.at,
        sequence: if entry.seq >= 0 {
            entry.seq
        } else {
            index as i64 + 1
        },
        label: entry.label.clone(),
        status: entry.status.clone(),
        redaction_state: if redacted { "masked" } else { "raw" }.to_string(),
        payload,
    }
}

fn redact_json(value: Value) -> (Value, usize) {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    let report = scan(&serialized, SecretLocation::Log, SecretAction::Masked);
    let redacted = serde_json::from_str(&report.text).unwrap_or(Value::String(report.text));
    (redacted, report.findings.len())
}

fn replay_summary(event: &TrajectoryEvent) -> String {
    let status = event.status.as_deref().unwrap_or("none");
    format!("{}:{} status={status}", event.event_type, event.label)
}

fn event_would_mutate(event: &TrajectoryEvent) -> bool {
    if event.event_type == "tool" {
        let text = event.payload.to_string().to_ascii_lowercase();
        return text.contains("write")
            || text.contains("edit")
            || text.contains("delete")
            || text.contains("commit")
            || text.contains("push")
            || text.contains("command");
    }
    matches!(event.event_type.as_str(), "permission" | "browser")
}

fn is_verification_event(event: &TrajectoryEvent) -> bool {
    let text = format!(
        "{} {} {}",
        event.event_type,
        event.label,
        event.status.as_deref().unwrap_or_default()
    )
    .to_ascii_lowercase();
    (text.contains("verify") || text.contains("verification") || text.contains("test"))
        && (text.contains("succeeded") || text.contains("success") || text.contains("finished"))
}

fn is_completion_claim(event: &TrajectoryEvent) -> bool {
    let text = format!(
        "{} {} {} {}",
        event.event_type,
        event.label,
        event.status.as_deref().unwrap_or_default(),
        event.payload
    )
    .to_ascii_lowercase();
    text.contains("completion")
        || text.contains("complete")
        || text.contains("completed")
        || text.contains("done")
        || text.contains("final")
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        LocalDb::open(std::env::temp_dir().join(format!("atlas_trajectory_{}.db", Uuid::new_v4())))
            .unwrap()
    }

    #[test]
    fn trajectory_exports_real_timeline_and_redacts_secrets() {
        let db = temp_db();
        let secret = format!("{}{}", "sk", "-proj-abcdefghijklmnopqrstuvwxyz");
        db.create_agent_run("run-traj", None, "default").unwrap();
        db.append_agent_run_step(
            "run-traj",
            "tool_call",
            "finished",
            "command finished",
            json!({ "command": format!("echo {secret}") }),
            json!({ "ok": true }),
        )
        .unwrap();
        let export = export_run_trajectory(
            &db,
            TrajectoryExportOptions {
                run_id: "run-traj".to_string(),
                include_payloads: true,
                redact_secrets: true,
                limit: None,
            },
        )
        .unwrap();
        assert_eq!(export.event_count, 1);
        assert_eq!(export.redaction_state, "masked");
        assert!(!export.events[0].payload.to_string().contains("sk-proj-"));
    }

    #[test]
    fn replay_is_readonly_and_marks_mutating_frames() {
        let export = TrajectoryExport {
            run_id: "run".to_string(),
            format: "atlas-trajectory-jsonl-v1".to_string(),
            event_count: 1,
            total_available: 1,
            truncated: false,
            redaction_state: "masked".to_string(),
            findings_redacted: 0,
            replay_safe: true,
            exported_at: 1,
            events: vec![TrajectoryEvent {
                event_type: "tool".to_string(),
                run_id: "run".to_string(),
                source_id: "tool-1".to_string(),
                timestamp: 1,
                sequence: 1,
                label: "write_file".to_string(),
                status: Some("finished".to_string()),
                redaction_state: "masked".to_string(),
                payload: json!({ "toolName": "write_file" }),
            }],
        };
        let replay = replay_trajectory_readonly(&export);
        assert!(replay.external_calls_blocked);
        assert!(replay.workspace_mutations_blocked);
        assert!(replay.frames[0].would_mutate);
    }

    #[test]
    fn trajectory_jsonl_import_rejects_mixed_run_ids() {
        let event_a = TrajectoryEvent {
            event_type: "step".to_string(),
            run_id: "run-a".to_string(),
            source_id: "s1".to_string(),
            timestamp: 1,
            sequence: 1,
            label: "plan".to_string(),
            status: Some("finished".to_string()),
            redaction_state: "masked".to_string(),
            payload: json!({}),
        };
        let event_b = TrajectoryEvent {
            run_id: "run-b".to_string(),
            ..event_a.clone()
        };
        let jsonl = format!(
            "{}\n{}",
            serde_json::to_string(&event_a).unwrap(),
            serde_json::to_string(&event_b).unwrap()
        );
        assert!(import_trajectory_jsonl(&jsonl, None)
            .unwrap_err()
            .contains("multiple run ids"));
    }

    #[test]
    fn trajectory_eval_flags_completion_without_verification() {
        let export = TrajectoryExport {
            run_id: "run-risk".to_string(),
            format: "atlas-trajectory-jsonl-v1".to_string(),
            event_count: 2,
            total_available: 2,
            truncated: false,
            redaction_state: "masked".to_string(),
            findings_redacted: 0,
            replay_safe: true,
            exported_at: 1,
            events: vec![
                TrajectoryEvent {
                    event_type: "tool".to_string(),
                    run_id: "run-risk".to_string(),
                    source_id: "tool-1".to_string(),
                    timestamp: 1,
                    sequence: 1,
                    label: "write_file".to_string(),
                    status: Some("finished".to_string()),
                    redaction_state: "masked".to_string(),
                    payload: json!({ "toolName": "write_file" }),
                },
                TrajectoryEvent {
                    event_type: "step".to_string(),
                    run_id: "run-risk".to_string(),
                    source_id: "step-2".to_string(),
                    timestamp: 2,
                    sequence: 2,
                    label: "final".to_string(),
                    status: Some("completed".to_string()),
                    redaction_state: "masked".to_string(),
                    payload: json!({ "message": "done" }),
                },
            ],
        };
        let report = evaluate_trajectory_completion(&export);
        assert_eq!(report.false_completion_risk, "high");
        assert_eq!(report.verification_frame_count, 0);
        assert!(report.completion_claim_count > 0);
    }
}
