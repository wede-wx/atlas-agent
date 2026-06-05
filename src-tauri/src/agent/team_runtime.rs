use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::storage::{
    AppendTeamMessagePayload, CreateHandoffRequestPayload, CreateTeamParticipantPayload,
    CreateTeamRunPayload, HandoffRequestRecord, LocalDb, StorageError, StorageResult,
    TeamMessageRecord, TeamRunSnapshot,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamParticipantSpec {
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tool_scope: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamRunSpec {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub source_run_id: Option<String>,
    pub goal: String,
    #[serde(default)]
    pub max_rounds: Option<i64>,
    #[serde(default)]
    pub participants: Vec<TeamParticipantSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffContract {
    pub task: String,
    #[serde(default)]
    pub expected_output: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub can_mark_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeamTerminationVerdict {
    pub status: String,
    pub reason: String,
    pub should_stop: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamExecutionOptions {
    #[serde(default)]
    pub max_steps: Option<usize>,
    #[serde(default)]
    pub default_token_budget: Option<i64>,
    #[serde(default)]
    pub role_token_budgets: BTreeMap<String, i64>,
    #[serde(default = "default_true")]
    pub require_main_review: bool,
}

impl Default for TeamExecutionOptions {
    fn default() -> Self {
        Self {
            max_steps: None,
            default_token_budget: None,
            role_token_budgets: BTreeMap::new(),
            require_main_review: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamExecutionStep {
    pub order: usize,
    pub participant_id: String,
    pub name: String,
    pub role: String,
    pub model: Option<String>,
    pub action: String,
    pub token_budget: i64,
    pub tool_scope: Value,
    pub status: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamExecutionPlan {
    pub team_run_id: String,
    pub run_status: String,
    pub paused: bool,
    pub requires_main_review: bool,
    pub step_count: usize,
    pub steps: Vec<TeamExecutionStep>,
    pub reason: String,
    pub built_at: i64,
}

#[derive(Debug, Clone)]
pub struct DurableTeamRuntime {
    db: LocalDb,
}

impl DurableTeamRuntime {
    pub fn new(db: LocalDb) -> Self {
        Self { db }
    }

    pub fn create_run(&self, spec: CreateTeamRunSpec) -> StorageResult<TeamRunSnapshot> {
        validate_team_spec(&spec)?;
        let run = self.db.create_team_run(CreateTeamRunPayload {
            id: spec.id,
            session_id: spec.session_id,
            source_run_id: spec.source_run_id,
            goal: spec.goal,
            max_rounds: spec.max_rounds,
        })?;
        for participant in spec.participants {
            self.db.add_team_participant(CreateTeamParticipantPayload {
                id: None,
                team_run_id: run.id.clone(),
                name: participant.name,
                role: participant.role,
                model: participant.model,
                tool_scope: participant.tool_scope,
            })?;
        }
        self.db.append_team_message(AppendTeamMessagePayload {
            id: None,
            team_run_id: run.id.clone(),
            participant_id: None,
            role: "system".to_string(),
            message_type: "task".to_string(),
            content: "team run created; subagent outputs require main-agent review".to_string(),
            metadata: json!({ "goal": run.goal, "maxRounds": run.max_rounds }),
        })?;
        self.db.get_team_run_snapshot(&run.id)
    }

    pub fn append_participant_message(
        &self,
        team_run_id: &str,
        participant_id: Option<String>,
        message_type: &str,
        content: &str,
        metadata: Value,
    ) -> StorageResult<TeamMessageRecord> {
        let snapshot = self.db.get_team_run_snapshot(team_run_id)?;
        let role = participant_id
            .as_deref()
            .and_then(|id| participant_role(&snapshot, id))
            .unwrap_or("main")
            .to_string();
        if role != "main"
            && matches!(
                message_type.trim(),
                "complete" | "completion" | "completion_claim"
            )
        {
            return Err(StorageError::Validation(
                "subagent messages cannot mark the team run complete; they must provide evidence or proposals"
                    .to_string(),
            ));
        }
        self.db.append_team_message(AppendTeamMessagePayload {
            id: None,
            team_run_id: team_run_id.to_string(),
            participant_id,
            role,
            message_type: message_type.to_string(),
            content: content.to_string(),
            metadata,
        })
    }

    pub fn request_handoff(
        &self,
        team_run_id: &str,
        from_participant_id: Option<String>,
        to_participant_id: String,
        reason: String,
        contract: HandoffContract,
    ) -> StorageResult<HandoffRequestRecord> {
        if contract.task.trim().is_empty() {
            return Err(StorageError::Validation(
                "handoff contract task is empty".to_string(),
            ));
        }
        self.db.create_handoff_request(CreateHandoffRequestPayload {
            id: None,
            team_run_id: team_run_id.to_string(),
            from_participant_id,
            to_participant_id,
            reason,
            contract: serde_json::to_value(contract).unwrap_or_else(|_| json!({})),
        })
    }

    pub fn evaluate_termination(&self, team_run_id: &str) -> StorageResult<TeamTerminationVerdict> {
        let snapshot = self.db.get_team_run_snapshot(team_run_id)?;
        Ok(team_termination_verdict(&snapshot))
    }

    pub fn apply_termination(&self, team_run_id: &str) -> StorageResult<TeamTerminationVerdict> {
        let verdict = self.evaluate_termination(team_run_id)?;
        if verdict.should_stop {
            self.db
                .update_team_run_status(team_run_id, &verdict.status, Some(&verdict.reason))?;
        }
        Ok(verdict)
    }

    pub fn build_execution_plan(
        &self,
        team_run_id: &str,
        options: TeamExecutionOptions,
    ) -> StorageResult<TeamExecutionPlan> {
        let snapshot = self.db.get_team_run_snapshot(team_run_id)?;
        Ok(team_execution_plan(&snapshot, options))
    }

    pub fn schedule_execution_plan(
        &self,
        team_run_id: &str,
        options: TeamExecutionOptions,
    ) -> StorageResult<TeamExecutionPlan> {
        let plan = self.build_execution_plan(team_run_id, options)?;
        self.db.append_team_message(AppendTeamMessagePayload {
            id: None,
            team_run_id: team_run_id.to_string(),
            participant_id: None,
            role: "system".to_string(),
            message_type: "status".to_string(),
            content: "team execution plan scheduled".to_string(),
            metadata: json!({
                "kind": "team_execution_plan",
                "plan": plan,
            }),
        })?;
        Ok(plan)
    }

    pub fn pause_execution(
        &self,
        team_run_id: &str,
        reason: &str,
    ) -> StorageResult<TeamExecutionPlan> {
        let reason = compact_reason(reason, "team execution paused");
        self.db
            .update_team_run_status(team_run_id, "paused", Some(&reason))?;
        self.db.append_team_message(AppendTeamMessagePayload {
            id: None,
            team_run_id: team_run_id.to_string(),
            participant_id: None,
            role: "system".to_string(),
            message_type: "status".to_string(),
            content: "team execution paused".to_string(),
            metadata: json!({ "kind": "team_control", "action": "pause", "reason": reason }),
        })?;
        self.build_execution_plan(team_run_id, TeamExecutionOptions::default())
    }

    pub fn resume_execution(
        &self,
        team_run_id: &str,
        reason: &str,
    ) -> StorageResult<TeamExecutionPlan> {
        let snapshot = self.db.get_team_run_snapshot(team_run_id)?;
        if matches!(
            snapshot.run.status.as_str(),
            "completed" | "failed" | "cancelled"
        ) {
            return Err(StorageError::Validation(
                "terminal team run cannot be resumed".to_string(),
            ));
        }
        let reason = compact_reason(reason, "team execution resumed");
        self.db
            .update_team_run_status(team_run_id, "running", Some(&reason))?;
        self.db.append_team_message(AppendTeamMessagePayload {
            id: None,
            team_run_id: team_run_id.to_string(),
            participant_id: None,
            role: "system".to_string(),
            message_type: "status".to_string(),
            content: "team execution resumed".to_string(),
            metadata: json!({ "kind": "team_control", "action": "resume", "reason": reason }),
        })?;
        self.build_execution_plan(team_run_id, TeamExecutionOptions::default())
    }
}

pub fn team_execution_plan(
    snapshot: &TeamRunSnapshot,
    options: TeamExecutionOptions,
) -> TeamExecutionPlan {
    let paused = snapshot.run.status == "paused";
    let terminal = matches!(
        snapshot.run.status.as_str(),
        "completed" | "failed" | "cancelled"
    );
    if paused || terminal {
        let reason = if paused {
            "team run is paused; no model work is scheduled"
        } else {
            "team run is terminal; no model work is scheduled"
        };
        return TeamExecutionPlan {
            team_run_id: snapshot.run.id.clone(),
            run_status: snapshot.run.status.clone(),
            paused,
            requires_main_review: options.require_main_review,
            step_count: 0,
            steps: Vec::new(),
            reason: reason.to_string(),
            built_at: chrono::Utc::now().timestamp_millis(),
        };
    }

    let mut participants = snapshot.participants.clone();
    participants.sort_by_key(|participant| {
        (
            role_priority(&participant.role),
            participant.created_at,
            participant.name.clone(),
        )
    });
    let max_steps = options.max_steps.unwrap_or(32).clamp(1, 128);
    let mut steps = Vec::new();
    for participant in participants.into_iter().take(max_steps) {
        let role = participant.role.clone();
        let token_budget = options
            .role_token_budgets
            .get(&role)
            .copied()
            .or(options.default_token_budget)
            .unwrap_or_else(|| default_budget_for_role(&role))
            .clamp(256, 200_000);
        let action = action_for_role(&role, options.require_main_review);
        let status = if role == "main" && options.require_main_review {
            "review_gate"
        } else {
            "ready"
        };
        steps.push(TeamExecutionStep {
            order: steps.len() + 1,
            participant_id: participant.id,
            name: participant.name,
            role: role.clone(),
            model: participant.model,
            action: action.to_string(),
            token_budget,
            tool_scope: participant.tool_scope,
            status: status.to_string(),
            reason: step_reason_for_role(&role, options.require_main_review).to_string(),
        });
    }

    TeamExecutionPlan {
        team_run_id: snapshot.run.id.clone(),
        run_status: snapshot.run.status.clone(),
        paused: false,
        requires_main_review: options.require_main_review,
        step_count: steps.len(),
        steps,
        reason: "team participants scheduled by durable role order".to_string(),
        built_at: chrono::Utc::now().timestamp_millis(),
    }
}

pub fn team_termination_verdict(snapshot: &TeamRunSnapshot) -> TeamTerminationVerdict {
    if snapshot
        .handoffs
        .iter()
        .any(|handoff| handoff.status == "pending" || handoff.status == "accepted")
    {
        return TeamTerminationVerdict {
            status: "running".to_string(),
            reason: "handoff still open".to_string(),
            should_stop: false,
        };
    }
    if snapshot.messages.iter().any(|message| {
        message.message_type == "main_review"
            && message
                .metadata
                .get("verified")
                .and_then(Value::as_bool)
                .unwrap_or(false)
    }) {
        return TeamTerminationVerdict {
            status: "completed".to_string(),
            reason: "main agent verified team evidence".to_string(),
            should_stop: true,
        };
    }
    if snapshot.messages.len() as i64 >= snapshot.run.max_rounds {
        return TeamTerminationVerdict {
            status: "blocked".to_string(),
            reason: "team reached max rounds without main-agent verified completion".to_string(),
            should_stop: true,
        };
    }
    TeamTerminationVerdict {
        status: "running".to_string(),
        reason: "waiting for main-agent review".to_string(),
        should_stop: false,
    }
}

fn participant_role<'a>(snapshot: &'a TeamRunSnapshot, participant_id: &str) -> Option<&'a str> {
    snapshot
        .participants
        .iter()
        .find(|participant| participant.id == participant_id)
        .map(|participant| participant.role.as_str())
}

fn validate_team_spec(spec: &CreateTeamRunSpec) -> StorageResult<()> {
    if spec.goal.trim().is_empty() {
        return Err(StorageError::Validation("team goal is empty".to_string()));
    }
    if spec.participants.is_empty() {
        return Err(StorageError::Validation(
            "team run requires at least one participant".to_string(),
        ));
    }
    if !spec
        .participants
        .iter()
        .any(|participant| participant.role.trim() == "main")
    {
        return Err(StorageError::Validation(
            "team run requires a main participant for final review".to_string(),
        ));
    }
    Ok(())
}

fn role_priority(role: &str) -> usize {
    match role {
        "planner" => 10,
        "researcher" => 20,
        "executor" => 30,
        "tester" => 40,
        "verifier" => 50,
        "reviewer" => 60,
        "main" => 90,
        _ => 70,
    }
}

fn default_budget_for_role(role: &str) -> i64 {
    match role {
        "planner" | "researcher" => 12_000,
        "executor" => 24_000,
        "tester" | "verifier" | "reviewer" => 10_000,
        "main" => 16_000,
        _ => 8_000,
    }
}

fn action_for_role(role: &str, require_main_review: bool) -> &'static str {
    match role {
        "planner" => "plan",
        "researcher" => "research",
        "executor" => "execute",
        "tester" => "test",
        "verifier" => "verify",
        "reviewer" => "review_evidence",
        "main" if require_main_review => "main_review_gate",
        "main" => "coordinate",
        _ => "assist",
    }
}

fn step_reason_for_role(role: &str, require_main_review: bool) -> &'static str {
    match role {
        "main" if require_main_review => {
            "main agent is scheduled last and is the only completion gate"
        }
        "verifier" | "tester" | "reviewer" => "verification role provides evidence, not completion",
        "executor" => "execution role performs scoped implementation work",
        "planner" => "planning role prepares the execution contract",
        _ => "participant scheduled within its stored tool scope",
    }
}

fn compact_reason(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.chars().take(500).collect()
    }
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        LocalDb::open(std::env::temp_dir().join(format!("atlas_team_{}.db", Uuid::new_v4())))
            .unwrap()
    }

    #[test]
    fn team_runtime_persists_roles_handoff_and_requires_main_review() {
        let runtime = DurableTeamRuntime::new(temp_db());
        let snapshot = runtime
            .create_run(CreateTeamRunSpec {
                id: Some("team-a".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "implement and verify".to_string(),
                max_rounds: Some(8),
                participants: vec![
                    TeamParticipantSpec {
                        name: "main".to_string(),
                        role: "main".to_string(),
                        model: None,
                        tool_scope: json!({}),
                    },
                    TeamParticipantSpec {
                        name: "verifier".to_string(),
                        role: "verifier".to_string(),
                        model: None,
                        tool_scope: json!({ "write": false }),
                    },
                ],
            })
            .unwrap();
        let verifier = snapshot
            .participants
            .iter()
            .find(|participant| participant.role == "verifier")
            .unwrap();
        runtime
            .request_handoff(
                "team-a",
                None,
                verifier.id.clone(),
                "verify evidence".to_string(),
                HandoffContract {
                    task: "verify".to_string(),
                    expected_output: "evidence only".to_string(),
                    allowed_tools: vec!["read_file".to_string()],
                    can_mark_complete: false,
                },
            )
            .unwrap();
        assert!(!runtime.evaluate_termination("team-a").unwrap().should_stop);
    }

    #[test]
    fn subagent_completion_claim_is_rejected_but_evidence_is_allowed() {
        let runtime = DurableTeamRuntime::new(temp_db());
        let snapshot = runtime
            .create_run(CreateTeamRunSpec {
                id: Some("team-b".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "verify only".to_string(),
                max_rounds: Some(4),
                participants: vec![
                    TeamParticipantSpec {
                        name: "main".to_string(),
                        role: "main".to_string(),
                        model: None,
                        tool_scope: json!({}),
                    },
                    TeamParticipantSpec {
                        name: "reviewer".to_string(),
                        role: "reviewer".to_string(),
                        model: None,
                        tool_scope: json!({}),
                    },
                ],
            })
            .unwrap();
        let reviewer = snapshot
            .participants
            .iter()
            .find(|participant| participant.role == "reviewer")
            .unwrap();
        assert!(runtime
            .append_participant_message(
                "team-b",
                Some(reviewer.id.clone()),
                "completion_claim",
                "done",
                json!({})
            )
            .is_err());
        runtime
            .append_participant_message(
                "team-b",
                Some(reviewer.id.clone()),
                "evidence",
                "tests pass",
                json!({ "command": "cargo test" }),
            )
            .unwrap();
    }

    #[test]
    fn main_review_is_the_only_completion_gate() {
        let runtime = DurableTeamRuntime::new(temp_db());
        runtime
            .create_run(CreateTeamRunSpec {
                id: Some("team-c".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "complete".to_string(),
                max_rounds: Some(10),
                participants: vec![TeamParticipantSpec {
                    name: "main".to_string(),
                    role: "main".to_string(),
                    model: None,
                    tool_scope: json!({}),
                }],
            })
            .unwrap();
        runtime
            .append_participant_message(
                "team-c",
                None,
                "main_review",
                "verified",
                json!({ "verified": true }),
            )
            .unwrap();
        let verdict = runtime.apply_termination("team-c").unwrap();
        assert_eq!(verdict.status, "completed");
        assert!(verdict.should_stop);
    }

    #[test]
    fn execution_plan_schedules_roles_with_budgets_and_main_review_last() {
        let runtime = DurableTeamRuntime::new(temp_db());
        runtime
            .create_run(CreateTeamRunSpec {
                id: Some("team-plan".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "plan execute verify".to_string(),
                max_rounds: Some(10),
                participants: vec![
                    TeamParticipantSpec {
                        name: "main".to_string(),
                        role: "main".to_string(),
                        model: Some("strong".to_string()),
                        tool_scope: json!({ "write": true }),
                    },
                    TeamParticipantSpec {
                        name: "executor".to_string(),
                        role: "executor".to_string(),
                        model: Some("coder".to_string()),
                        tool_scope: json!({ "write": true }),
                    },
                    TeamParticipantSpec {
                        name: "planner".to_string(),
                        role: "planner".to_string(),
                        model: None,
                        tool_scope: json!({ "write": false }),
                    },
                    TeamParticipantSpec {
                        name: "verifier".to_string(),
                        role: "verifier".to_string(),
                        model: None,
                        tool_scope: json!({ "write": false }),
                    },
                ],
            })
            .unwrap();
        let mut budgets = BTreeMap::new();
        budgets.insert("executor".to_string(), 50_000);
        let plan = runtime
            .schedule_execution_plan(
                "team-plan",
                TeamExecutionOptions {
                    role_token_budgets: budgets,
                    ..TeamExecutionOptions::default()
                },
            )
            .unwrap();
        let roles = plan
            .steps
            .iter()
            .map(|step| step.role.as_str())
            .collect::<Vec<_>>();
        assert_eq!(roles, ["planner", "executor", "verifier", "main"]);
        assert_eq!(plan.steps[1].token_budget, 50_000);
        assert_eq!(plan.steps.last().unwrap().status, "review_gate");
        let snapshot = runtime.db.get_team_run_snapshot("team-plan").unwrap();
        assert!(snapshot
            .messages
            .iter()
            .any(|message| message.content == "team execution plan scheduled"));
    }

    #[test]
    fn pause_and_resume_control_team_execution_queue() {
        let runtime = DurableTeamRuntime::new(temp_db());
        runtime
            .create_run(CreateTeamRunSpec {
                id: Some("team-pause".to_string()),
                session_id: None,
                source_run_id: None,
                goal: "pause".to_string(),
                max_rounds: Some(10),
                participants: vec![TeamParticipantSpec {
                    name: "main".to_string(),
                    role: "main".to_string(),
                    model: None,
                    tool_scope: json!({}),
                }],
            })
            .unwrap();
        let paused = runtime
            .pause_execution("team-pause", "operator pause")
            .unwrap();
        assert!(paused.paused);
        assert!(paused.steps.is_empty());
        let resumed = runtime
            .resume_execution("team-pause", "operator resume")
            .unwrap();
        assert!(!resumed.paused);
        assert_eq!(resumed.steps.len(), 1);
    }
}
