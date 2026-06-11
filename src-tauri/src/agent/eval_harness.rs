use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::storage::{
    EvalRunStorageRecord, LocalDb, PersistEvalCaseResultPayload, PersistEvalCommandResultPayload,
    PersistEvalRunPayload,
};

const BENCHMARK_SUITE: &str = include_str!("eval_suites/benchmark.json");
const SECURITY_SUITE: &str = include_str!("eval_suites/security_attacks.json");
const FALSE_COMPLETION_SUITE: &str = include_str!("eval_suites/false_completion.json");
const ROLLBACK_SUITE: &str = include_str!("eval_suites/rollback.json");
const PROVIDER_COMPAT_SUITE: &str = include_str!("eval_suites/provider_compat.json");
/// Step 6：五步加固闸的回归套件——每个 case 绑定对应步交付的确定性测试。
const ATLAS_GATES_SUITE: &str = include_str!("eval_suites/atlas_gates.json");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalSuite {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub description: String,
    pub exit_gate: EvalExitGate,
    pub cases: Vec<EvalCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalExitGate {
    pub min_cases: usize,
    pub min_pass_rate: f64,
    pub max_false_completion_rate: f64,
    pub require_all_critical: bool,
    #[serde(default)]
    pub required_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalCase {
    pub id: String,
    pub title: String,
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub prompt: String,
    #[serde(default)]
    pub setup: Vec<String>,
    #[serde(default)]
    pub allowed_providers: Vec<String>,
    #[serde(default)]
    pub expected: Vec<String>,
    #[serde(default)]
    pub forbidden: Vec<String>,
    pub verifier: EvalVerifier,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalVerifier {
    #[serde(default)]
    pub commands: Vec<EvalCommand>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalCommand {
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default = "default_required")]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalCaseOutcome {
    pub case_id: String,
    pub passed: bool,
    pub verified: bool,
    #[serde(default)]
    pub false_completion: bool,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalSuiteReport {
    pub suite_id: String,
    pub total_cases: usize,
    pub evaluated_cases: usize,
    pub passed_cases: usize,
    pub verified_cases: usize,
    pub false_completion_cases: usize,
    pub blocked_cases: usize,
    pub pass_rate: f64,
    pub false_completion_rate: f64,
    pub missing_outcomes: Vec<String>,
    pub unknown_outcomes: Vec<String>,
    pub critical_failures: Vec<String>,
    pub gate_failures: Vec<String>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalRunOptions {
    pub suite_id: String,
    #[serde(default)]
    pub case_ids: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub claimed_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalRunReport {
    pub id: String,
    pub suite_id: String,
    pub started_at: i64,
    pub finished_at: i64,
    pub duration_ms: i64,
    pub cwd: String,
    pub case_results: Vec<EvalCaseRunResult>,
    pub score: EvalSuiteReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalCaseRunResult {
    pub case_id: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub status: String,
    pub outcome: EvalCaseOutcome,
    pub case_artifact_path: String,
    pub case_artifact_sha256: String,
    #[serde(default)]
    pub agent_result: Option<EvalAgentCaseResult>,
    pub commands: Vec<EvalCommandResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalAgentCaseResult {
    pub status: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub output: Value,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalCommandResult {
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

pub fn builtin_eval_suites() -> Result<Vec<EvalSuite>, String> {
    let suites = [
        ("benchmark", BENCHMARK_SUITE),
        ("security_attacks", SECURITY_SUITE),
        ("false_completion", FALSE_COMPLETION_SUITE),
        ("rollback", ROLLBACK_SUITE),
        ("provider_compat", PROVIDER_COMPAT_SUITE),
        ("atlas_gates", ATLAS_GATES_SUITE),
    ];
    let mut parsed = Vec::new();
    for (name, text) in suites {
        let suite: EvalSuite =
            serde_json::from_str(text).map_err(|error| format!("{name} suite JSON: {error}"))?;
        validate_eval_suite(&suite)
            .map_err(|errors| format!("{} suite invalid: {}", suite.id, errors.join("; ")))?;
        parsed.push(suite);
    }
    Ok(parsed)
}

pub fn builtin_eval_suite(suite_id: &str) -> Result<EvalSuite, String> {
    builtin_eval_suites()?
        .into_iter()
        .find(|suite| suite.id == suite_id)
        .ok_or_else(|| format!("unknown eval suite: {suite_id}"))
}

pub fn validate_eval_suite(suite: &EvalSuite) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    if suite.id.trim().is_empty() {
        errors.push("suite id is required".to_string());
    }
    if suite.name.trim().is_empty() {
        errors.push("suite name is required".to_string());
    }
    if suite.kind.trim().is_empty() {
        errors.push("suite kind is required".to_string());
    }
    if suite.cases.len() < suite.exit_gate.min_cases {
        errors.push(format!(
            "case count {} is below min_cases {}",
            suite.cases.len(),
            suite.exit_gate.min_cases
        ));
    }
    if !(0.0..=1.0).contains(&suite.exit_gate.min_pass_rate) {
        errors.push("min_pass_rate must be 0..1".to_string());
    }
    if !(0.0..=1.0).contains(&suite.exit_gate.max_false_completion_rate) {
        errors.push("max_false_completion_rate must be 0..1".to_string());
    }

    let mut ids = BTreeSet::new();
    let mut tags = BTreeSet::new();
    let mut providers = BTreeSet::new();
    for case in &suite.cases {
        if !ids.insert(case.id.clone()) {
            errors.push(format!("duplicate case id: {}", case.id));
        }
        if case.title.trim().is_empty() {
            errors.push(format!("{} title is required", case.id));
        }
        if case.prompt.trim().is_empty() {
            errors.push(format!("{} prompt is required", case.id));
        }
        if case.expected.is_empty() {
            errors.push(format!("{} expected outcomes are required", case.id));
        }
        if case.forbidden.is_empty() {
            errors.push(format!("{} forbidden outcomes are required", case.id));
        }
        if case.verifier.commands.is_empty()
            && case.verifier.evidence.is_empty()
            && case.verifier.success_criteria.is_empty()
        {
            errors.push(format!("{} needs verifier evidence or commands", case.id));
        }
        if case.verifier.evidence.is_empty() {
            errors.push(format!("{} verifier evidence is required", case.id));
        }
        if case.verifier.success_criteria.is_empty() {
            errors.push(format!(
                "{} verifier success criteria are required",
                case.id
            ));
        }
        for command in &case.verifier.commands {
            if command.command.trim().is_empty() {
                errors.push(format!("{} has an empty verifier command", case.id));
            }
        }
        for evidence in &case.verifier.evidence {
            if evidence.trim().is_empty() {
                errors.push(format!("{} has empty verifier evidence", case.id));
            }
        }
        for criterion in &case.verifier.success_criteria {
            if criterion.trim().is_empty() {
                errors.push(format!("{} has empty success criterion", case.id));
            }
        }
        for tag in &case.tags {
            tags.insert(tag.clone());
        }
        for provider in &case.allowed_providers {
            providers.insert(provider.clone());
        }
    }

    for required in &suite.exit_gate.required_tags {
        if !tags.contains(required) {
            errors.push(format!("missing required tag: {required}"));
        }
    }

    match suite.kind.as_str() {
        "benchmark" => {
            if suite.cases.len() < 10 {
                errors.push("benchmark suite must contain at least 10 cases".to_string());
            }
        }
        "provider_compat" => {
            if providers.len() < 4 {
                errors.push(
                    "provider compatibility suite must cover at least 4 providers".to_string(),
                );
            }
        }
        "security_attacks" | "false_completion" | "rollback" | "atlas_gates" => {}
        other => errors.push(format!("unknown suite kind: {other}")),
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn score_eval_suite(suite: &EvalSuite, outcomes: &[EvalCaseOutcome]) -> EvalSuiteReport {
    let mut outcomes_by_id: BTreeMap<&str, &EvalCaseOutcome> = BTreeMap::new();
    let case_ids: BTreeSet<&str> = suite.cases.iter().map(|case| case.id.as_str()).collect();
    let mut unknown_outcomes = Vec::new();
    for outcome in outcomes {
        if case_ids.contains(outcome.case_id.as_str()) {
            outcomes_by_id.insert(outcome.case_id.as_str(), outcome);
        } else {
            unknown_outcomes.push(outcome.case_id.clone());
        }
    }

    let mut missing_outcomes = Vec::new();
    let mut critical_failures = Vec::new();
    let mut passed_cases = 0;
    let mut verified_cases = 0;
    let mut false_completion_cases = 0;
    let mut blocked_cases = 0;

    for case in &suite.cases {
        let Some(outcome) = outcomes_by_id.get(case.id.as_str()) else {
            missing_outcomes.push(case.id.clone());
            if case.tags.iter().any(|tag| tag == "critical") {
                critical_failures.push(case.id.clone());
            }
            continue;
        };

        if outcome.verified {
            verified_cases += 1;
        }
        if outcome.false_completion {
            false_completion_cases += 1;
        }
        if outcome.blocked {
            blocked_cases += 1;
        }
        let case_passed =
            outcome.passed && outcome.verified && !outcome.false_completion && !outcome.blocked;
        if case_passed {
            passed_cases += 1;
        } else if case.tags.iter().any(|tag| tag == "critical") {
            critical_failures.push(case.id.clone());
        }
    }

    let total_cases = suite.cases.len();
    let pass_rate = rate(passed_cases, total_cases);
    let false_completion_rate = rate(false_completion_cases, total_cases);
    let mut gate_failures = Vec::new();
    if !missing_outcomes.is_empty() {
        gate_failures.push(format!("{} missing case outcomes", missing_outcomes.len()));
    }
    if !unknown_outcomes.is_empty() {
        gate_failures.push(format!("{} unknown case outcomes", unknown_outcomes.len()));
    }
    if pass_rate + f64::EPSILON < suite.exit_gate.min_pass_rate {
        gate_failures.push(format!(
            "pass rate {:.3} below gate {:.3}",
            pass_rate, suite.exit_gate.min_pass_rate
        ));
    }
    if false_completion_rate > suite.exit_gate.max_false_completion_rate + f64::EPSILON {
        gate_failures.push(format!(
            "false completion rate {:.3} above gate {:.3}",
            false_completion_rate, suite.exit_gate.max_false_completion_rate
        ));
    }
    if suite.exit_gate.require_all_critical && !critical_failures.is_empty() {
        gate_failures.push(format!("{} critical cases failed", critical_failures.len()));
    }

    EvalSuiteReport {
        suite_id: suite.id.clone(),
        total_cases,
        evaluated_cases: outcomes_by_id.len(),
        passed_cases,
        verified_cases,
        false_completion_cases,
        blocked_cases,
        pass_rate,
        false_completion_rate,
        missing_outcomes,
        unknown_outcomes,
        critical_failures,
        passed: gate_failures.is_empty(),
        gate_failures,
    }
}

pub fn run_eval_suite_verifiers(options: EvalRunOptions) -> Result<EvalRunReport, String> {
    let suite = builtin_eval_suite(&options.suite_id)?;
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let executor = ProcessEvalCommandExecutor;
    run_eval_suite_verifiers_with_executor(&suite, &options, &cwd, &executor)
}

pub trait EvalCommandExecutor {
    fn run(&self, command: &EvalCommand, cwd: &Path) -> EvalCommandResult;
}

pub trait EvalAgentExecutor {
    fn run_case(&self, case: &EvalCase, artifact_path: &Path) -> EvalAgentCaseResult;
}

pub struct NoopEvalAgentExecutor;

impl EvalAgentExecutor for NoopEvalAgentExecutor {
    fn run_case(&self, _case: &EvalCase, _artifact_path: &Path) -> EvalAgentCaseResult {
        EvalAgentCaseResult {
            status: "skipped".to_string(),
            provider: None,
            model: None,
            output: json!({}),
            notes: Some("agent execution was not requested for this verifier-only run".to_string()),
        }
    }
}

pub fn run_eval_suite_verifiers_with_executor<E: EvalCommandExecutor>(
    suite: &EvalSuite,
    options: &EvalRunOptions,
    cwd: &Path,
    executor: &E,
) -> Result<EvalRunReport, String> {
    let agent = NoopEvalAgentExecutor;
    run_eval_suite_with_executors(suite, options, cwd, &agent, executor, false)
}

#[allow(clippy::too_many_arguments)]
pub fn run_eval_orchestration_with_executors<A, E>(
    db: &LocalDb,
    suite: &EvalSuite,
    options: &EvalRunOptions,
    cwd: &Path,
    agent_executor: &A,
    command_executor: &E,
    provider: Option<String>,
    model: Option<String>,
) -> Result<EvalRunReport, String>
where
    A: EvalAgentExecutor,
    E: EvalCommandExecutor,
{
    let report =
        run_eval_suite_with_executors(suite, options, cwd, agent_executor, command_executor, true)?;
    persist_eval_report(db, &report, provider.as_deref(), model.as_deref())?;
    Ok(report)
}

pub fn persist_eval_report(
    db: &LocalDb,
    report: &EvalRunReport,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<EvalRunStorageRecord, String> {
    let cases = report
        .case_results
        .iter()
        .map(|case| PersistEvalCaseResultPayload {
            eval_run_id: report.id.clone(),
            case_id: case.case_id.clone(),
            status: case.status.clone(),
            passed: case.outcome.passed,
            verified: case.outcome.verified,
            false_completion: case.outcome.false_completion,
            blocked: case.outcome.blocked,
            artifact_path: Some(case.case_artifact_path.clone()),
            result: serde_json::to_value(case).unwrap_or_else(|_| json!({})),
        })
        .collect::<Vec<_>>();
    let commands = report
        .case_results
        .iter()
        .flat_map(|case| {
            case.commands
                .iter()
                .map(|command| PersistEvalCommandResultPayload {
                    eval_run_id: report.id.clone(),
                    case_id: case.case_id.clone(),
                    command: command.command.clone(),
                    cwd: command.cwd.clone(),
                    required: command.required,
                    status: command.status.clone(),
                    exit_code: command.exit_code,
                    stdout_tail: command.stdout_tail.clone(),
                    stderr_tail: command.stderr_tail.clone(),
                    started_at: command.started_at,
                    finished_at: command.finished_at,
                    duration_ms: command.duration_ms,
                    timed_out: command.timed_out,
                })
        })
        .collect::<Vec<_>>();
    db.persist_eval_run_report(
        PersistEvalRunPayload {
            id: report.id.clone(),
            suite_id: report.suite_id.clone(),
            provider: provider.map(str::to_string),
            model: model.map(str::to_string),
            status: if report.score.passed {
                "passed".to_string()
            } else {
                "failed".to_string()
            },
            cwd: report.cwd.clone(),
            passed: report.score.passed,
            started_at: report.started_at,
            finished_at: report.finished_at,
            report: serde_json::to_value(report).unwrap_or_else(|_| json!({})),
        },
        cases,
        commands,
    )
    .map_err(|error| error.to_string())
}

fn run_eval_suite_with_executors<A, E>(
    suite: &EvalSuite,
    options: &EvalRunOptions,
    cwd: &Path,
    agent_executor: &A,
    command_executor: &E,
    run_agent: bool,
) -> Result<EvalRunReport, String>
where
    A: EvalAgentExecutor,
    E: EvalCommandExecutor,
{
    validate_eval_suite(suite)
        .map_err(|errors| format!("{} suite invalid: {}", suite.id, errors.join("; ")))?;
    let selected = selected_eval_cases(suite, &options.case_ids)?;
    let run_id = format!("eval_{}", Uuid::new_v4());
    let started_at = now_ms();
    let timer = Instant::now();
    let mut case_results = Vec::new();
    let mut outcomes = Vec::new();

    for case in selected {
        let (case_artifact_path, case_artifact_sha256) =
            write_eval_case_artifact(cwd, &run_id, case)?;
        let agent_result =
            run_agent.then(|| agent_executor.run_case(case, Path::new(&case_artifact_path)));
        let mut command_results = Vec::new();
        for command in &case.verifier.commands {
            let command_cwd = resolve_command_cwd(cwd, command.cwd.as_deref());
            command_results.push(command_executor.run(command, &command_cwd));
        }

        let required: Vec<&EvalCommandResult> = command_results
            .iter()
            .filter(|result| result.required)
            .collect();
        let agent_blocked = agent_result
            .as_ref()
            .is_some_and(|result| matches!(result.status.as_str(), "blocked" | "timeout"));
        let agent_failed = agent_result
            .as_ref()
            .is_some_and(|result| matches!(result.status.as_str(), "failed" | "error"));
        let agent_passed = agent_result.as_ref().is_none_or(|result| {
            matches!(result.status.as_str(), "passed" | "succeeded" | "completed")
        });
        let blocked = agent_blocked || required.iter().any(|result| result.timed_out);
        let verified = !required.is_empty()
            && Path::new(&case_artifact_path).is_file()
            && required
                .iter()
                .all(|result| matches!(result.status.as_str(), "passed" | "failed"));
        let passed = verified
            && agent_passed
            && required
                .iter()
                .all(|result| result.status == "passed" && result.exit_code == Some(0));
        let false_completion = options.claimed_complete && !passed;
        let status = if blocked {
            "blocked"
        } else if passed {
            "passed"
        } else if agent_failed || verified {
            "failed"
        } else {
            "unverified"
        }
        .to_string();
        let notes = if command_results.is_empty() {
            Some("case has no executable verifier commands".to_string())
        } else if agent_failed {
            agent_result
                .as_ref()
                .and_then(|result| result.notes.clone())
                .or_else(|| Some("agent case execution failed".to_string()))
        } else {
            None
        };
        let outcome = EvalCaseOutcome {
            case_id: case.id.clone(),
            passed,
            verified,
            false_completion,
            blocked,
            provider: agent_result
                .as_ref()
                .and_then(|result| result.provider.clone()),
            notes,
        };
        outcomes.push(outcome.clone());
        case_results.push(EvalCaseRunResult {
            case_id: case.id.clone(),
            title: case.title.clone(),
            category: case.category.clone(),
            tags: case.tags.clone(),
            status,
            outcome,
            case_artifact_path,
            case_artifact_sha256,
            agent_result,
            commands: command_results,
        });
    }

    let score = score_eval_suite(suite, &outcomes);
    let finished_at = now_ms();
    Ok(EvalRunReport {
        id: run_id,
        suite_id: suite.id.clone(),
        started_at,
        finished_at,
        duration_ms: timer.elapsed().as_millis().min(i64::MAX as u128) as i64,
        cwd: cwd.to_string_lossy().to_string(),
        case_results,
        score,
    })
}

pub struct ProcessEvalCommandExecutor;

impl EvalCommandExecutor for ProcessEvalCommandExecutor {
    fn run(&self, command: &EvalCommand, cwd: &Path) -> EvalCommandResult {
        let started_at = now_ms();
        let timer = Instant::now();
        let timeout =
            Duration::from_millis(command.timeout_ms.unwrap_or(300_000).clamp(1_000, 900_000));
        let stdout_path = eval_output_path("stdout");
        let stderr_path = eval_output_path("stderr");
        let stdout_handle = match File::create(&stdout_path) {
            Ok(file) => file,
            Err(error) => {
                return failed_command_result(command, cwd, started_at, &timer, error.to_string());
            }
        };
        let stderr_handle = match File::create(&stderr_path) {
            Ok(file) => file,
            Err(error) => {
                cleanup_eval_output_files(&stdout_path, &stderr_path);
                return failed_command_result(command, cwd, started_at, &timer, error.to_string());
            }
        };
        let mut child = match shell_command(&command.command)
            .current_dir(cwd)
            .stdout(Stdio::from(stdout_handle))
            .stderr(Stdio::from(stderr_handle))
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                cleanup_eval_output_files(&stdout_path, &stderr_path);
                return failed_command_result(command, cwd, started_at, &timer, error.to_string());
            }
        };

        let mut timed_out = false;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if timer.elapsed() >= timeout => {
                    timed_out = true;
                    let _ = child.kill();
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(error) => {
                    cleanup_eval_output_files(&stdout_path, &stderr_path);
                    return failed_command_result(
                        command,
                        cwd,
                        started_at,
                        &timer,
                        error.to_string(),
                    );
                }
            }
        }

        let output = child.wait();
        let finished_at = now_ms();
        match output {
            Ok(status) => {
                let exit_code = status.code().map(i64::from);
                let stdout_tail = read_tail_lossy(&stdout_path, 4000);
                let stderr_tail = read_tail_lossy(&stderr_path, 4000);
                cleanup_eval_output_files(&stdout_path, &stderr_path);
                EvalCommandResult {
                    command: command.command.clone(),
                    cwd: cwd.to_string_lossy().to_string(),
                    required: command.required,
                    status: if timed_out {
                        "timeout".to_string()
                    } else if status.success() {
                        "passed".to_string()
                    } else {
                        "failed".to_string()
                    },
                    exit_code,
                    stdout_tail,
                    stderr_tail,
                    started_at,
                    finished_at,
                    duration_ms: timer.elapsed().as_millis().min(i64::MAX as u128) as i64,
                    timed_out,
                }
            }
            Err(error) => {
                cleanup_eval_output_files(&stdout_path, &stderr_path);
                failed_command_result(command, cwd, started_at, &timer, error.to_string())
            }
        }
    }
}

fn write_eval_case_artifact(
    cwd: &Path,
    run_id: &str,
    case: &EvalCase,
) -> Result<(String, String), String> {
    let dir = cwd.join("target").join("atlas-eval").join(run_id);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(format!("{}.json", safe_path_segment(&case.id)));
    let bytes = serde_json::to_vec_pretty(&json!({
        "caseId": &case.id,
        "title": &case.title,
        "category": &case.category,
        "tags": &case.tags,
        "prompt": &case.prompt,
        "setup": &case.setup,
        "allowedProviders": &case.allowed_providers,
        "expected": &case.expected,
        "forbidden": &case.forbidden,
        "verifier": &case.verifier,
        "artifactVersion": 1
    }))
    .map_err(|error| error.to_string())?;
    std::fs::write(&path, &bytes).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok((
        path.to_string_lossy().to_string(),
        format!("{:x}", hasher.finalize()),
    ))
}

fn rate(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn default_required() -> bool {
    true
}

fn selected_eval_cases<'a>(
    suite: &'a EvalSuite,
    case_ids: &[String],
) -> Result<Vec<&'a EvalCase>, String> {
    if case_ids.is_empty() {
        return Ok(suite.cases.iter().collect());
    }
    let requested: BTreeSet<&str> = case_ids.iter().map(String::as_str).collect();
    let known: BTreeSet<&str> = suite.cases.iter().map(|case| case.id.as_str()).collect();
    let unknown: Vec<&str> = requested.difference(&known).copied().collect();
    if !unknown.is_empty() {
        return Err(format!("unknown eval case ids: {}", unknown.join(", ")));
    }
    Ok(suite
        .cases
        .iter()
        .filter(|case| requested.contains(case.id.as_str()))
        .collect())
}

fn safe_path_segment(value: &str) -> String {
    let segment = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if segment.trim_matches('_').is_empty() {
        "case".to_string()
    } else {
        segment
    }
}

fn resolve_command_cwd(base: &Path, cwd: Option<&str>) -> PathBuf {
    match cwd.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                base.join(path)
            }
        }
        None => base.to_path_buf(),
    }
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("powershell");
        cmd.arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-Command")
            .arg(command);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn tail_text(value: &str, max_chars: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max_chars {
        value.to_string()
    } else {
        chars[chars.len() - max_chars..].iter().collect()
    }
}

fn eval_output_path(kind: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "atlas_eval_{}_{}_{}.log",
        std::process::id(),
        Uuid::new_v4(),
        kind
    ))
}

fn cleanup_eval_output_files(stdout_path: &Path, stderr_path: &Path) {
    let _ = std::fs::remove_file(stdout_path);
    let _ = std::fs::remove_file(stderr_path);
}

fn read_tail_lossy(path: &Path, max_chars: usize) -> String {
    match std::fs::read(path) {
        Ok(bytes) => tail_text(&String::from_utf8_lossy(&bytes), max_chars),
        Err(error) => format!("failed to read verifier output: {error}"),
    }
}

fn failed_command_result(
    command: &EvalCommand,
    cwd: &Path,
    started_at: i64,
    timer: &Instant,
    stderr_tail: String,
) -> EvalCommandResult {
    let finished_at = now_ms();
    EvalCommandResult {
        command: command.command.clone(),
        cwd: cwd.to_string_lossy().to_string(),
        required: command.required,
        status: "failed".to_string(),
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail,
        started_at,
        finished_at,
        duration_ms: timer.elapsed().as_millis().min(i64::MAX as u128) as i64,
        timed_out: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeExecutor {
        statuses: BTreeMap<String, EvalCommandResult>,
    }

    impl EvalCommandExecutor for FakeExecutor {
        fn run(&self, command: &EvalCommand, cwd: &Path) -> EvalCommandResult {
            self.statuses
                .get(&command.command)
                .cloned()
                .unwrap_or_else(|| EvalCommandResult {
                    command: command.command.clone(),
                    cwd: cwd.to_string_lossy().to_string(),
                    required: command.required,
                    status: "passed".to_string(),
                    exit_code: Some(0),
                    stdout_tail: String::new(),
                    stderr_tail: String::new(),
                    started_at: 1,
                    finished_at: 2,
                    duration_ms: 1,
                    timed_out: false,
                })
        }
    }

    struct PassingAgent;

    impl EvalAgentExecutor for PassingAgent {
        fn run_case(&self, case: &EvalCase, artifact_path: &Path) -> EvalAgentCaseResult {
            EvalAgentCaseResult {
                status: "passed".to_string(),
                provider: Some("openai".to_string()),
                model: Some("gpt-4o-mini".to_string()),
                output: json!({
                    "caseId": &case.id,
                    "artifact": artifact_path.to_string_lossy()
                }),
                notes: None,
            }
        }
    }

    fn temp_db() -> LocalDb {
        LocalDb::open(
            std::env::temp_dir().join(format!("atlas_eval_harness_{}.db", Uuid::new_v4())),
        )
        .unwrap()
    }

    #[test]
    fn builtin_eval_suites_are_valid() {
        let suites = builtin_eval_suites().unwrap();
        assert_eq!(suites.len(), 6);
    }

    #[test]
    fn benchmark_suite_has_real_exit_gate_and_ten_cases() {
        let suite = builtin_eval_suite("benchmark_core").unwrap();
        assert!(suite.cases.len() >= 10);
        assert!(suite.exit_gate.min_pass_rate >= 0.8);
        assert!(suite
            .exit_gate
            .required_tags
            .iter()
            .any(|tag| tag == "real_verification"));
    }

    #[test]
    fn atlas_gates_suite_pins_every_hardening_step() {
        // Step 6：套件本身也要被钉死——出口闸必须是 1.0 全绿（用例全部绑定
        // 确定性测试，红了就是闸回归，不存在"模型不稳定"的借口），且五步
        // 对应的关键 tag 一个都不能少。
        let suite = builtin_eval_suite("atlas_gates_suite").unwrap();
        assert!(suite.cases.len() >= 10);
        assert_eq!(suite.exit_gate.min_pass_rate, 1.0);
        assert_eq!(suite.exit_gate.max_false_completion_rate, 0.0);
        assert!(suite.exit_gate.require_all_critical);
        for tag in [
            "contract_channel",
            "deviation_approval",
            "done_gate",
            "sandbox",
        ] {
            assert!(
                suite.exit_gate.required_tags.iter().any(|t| t == tag),
                "required tag {tag} missing from exit gate"
            );
        }
        // 每个 case 的 verifier 命令都必须指向真实的 cargo test 过滤器。
        for case in &suite.cases {
            assert!(
                case.verifier
                    .commands
                    .iter()
                    .all(|cmd| cmd.command.starts_with("cargo test --lib ")),
                "{} must verify via deterministic cargo tests",
                case.id
            );
        }
    }

    #[test]
    fn scoring_blocks_false_completion_even_when_case_says_passed() {
        let suite = builtin_eval_suite("false_completion_guard").unwrap();
        let outcomes: Vec<EvalCaseOutcome> = suite
            .cases
            .iter()
            .map(|case| EvalCaseOutcome {
                case_id: case.id.clone(),
                passed: true,
                verified: true,
                false_completion: case.id == "fc-claims-tests-without-running",
                blocked: false,
                provider: None,
                notes: None,
            })
            .collect();

        let report = score_eval_suite(&suite, &outcomes);
        assert!(!report.passed);
        assert_eq!(report.false_completion_cases, 1);
        assert!(report
            .gate_failures
            .iter()
            .any(|failure| failure.contains("false completion rate")));
    }

    #[test]
    fn provider_suite_covers_multiple_provider_shapes() {
        let suite = builtin_eval_suite("provider_compat_matrix").unwrap();
        let providers: BTreeSet<&str> = suite
            .cases
            .iter()
            .flat_map(|case| case.allowed_providers.iter().map(String::as_str))
            .collect();
        assert!(providers.contains("openai"));
        assert!(providers.contains("anthropic"));
        assert!(providers.contains("ollama"));
        assert!(providers.contains("lmstudio"));
    }

    #[test]
    fn eval_runner_executes_verifiers_and_scores_exit_gate() {
        let suite = builtin_eval_suite("false_completion_guard").unwrap();
        let first = suite.cases[0].id.clone();
        let options = EvalRunOptions {
            suite_id: suite.id.clone(),
            case_ids: vec![first.clone()],
            cwd: None,
            claimed_complete: true,
        };
        let report = run_eval_suite_verifiers_with_executor(
            &suite,
            &options,
            Path::new("."),
            &FakeExecutor {
                statuses: BTreeMap::new(),
            },
        )
        .unwrap();

        assert_eq!(report.case_results.len(), 1);
        assert_eq!(report.case_results[0].status, "passed");
        assert!(
            !report.score.passed,
            "partial suite runs must still fail the full-suite gate because outcomes are missing"
        );
        assert!(report.score.missing_outcomes.len() >= suite.cases.len() - 1);
        assert!(Path::new(&report.case_results[0].case_artifact_path).is_file());
        assert!(!report.case_results[0].case_artifact_sha256.is_empty());
    }

    #[test]
    fn eval_runner_marks_claimed_completion_false_when_verifier_fails() {
        let suite = builtin_eval_suite("false_completion_guard").unwrap();
        let case = &suite.cases[0];
        let failing = EvalCommandResult {
            command: case.verifier.commands[0].command.clone(),
            cwd: ".".to_string(),
            required: true,
            status: "failed".to_string(),
            exit_code: Some(1),
            stdout_tail: String::new(),
            stderr_tail: "boom".to_string(),
            started_at: 1,
            finished_at: 2,
            duration_ms: 1,
            timed_out: false,
        };
        let mut statuses = BTreeMap::new();
        statuses.insert(case.verifier.commands[0].command.clone(), failing);
        let options = EvalRunOptions {
            suite_id: suite.id.clone(),
            case_ids: vec![case.id.clone()],
            cwd: None,
            claimed_complete: true,
        };
        let report = run_eval_suite_verifiers_with_executor(
            &suite,
            &options,
            Path::new("."),
            &FakeExecutor { statuses },
        )
        .unwrap();

        let result = &report.case_results[0];
        assert_eq!(result.status, "failed");
        assert!(!result.outcome.passed);
        assert!(result.outcome.verified);
        assert!(result.outcome.false_completion);
    }

    #[test]
    fn process_eval_executor_runs_real_command_and_captures_output() {
        let command_text = if cfg!(windows) {
            "Write-Output eval_runner_real_process"
        } else {
            "printf eval_runner_real_process"
        };
        let command = EvalCommand {
            command: command_text.to_string(),
            cwd: None,
            timeout_ms: Some(10_000),
            required: true,
        };
        let result = ProcessEvalCommandExecutor.run(&command, Path::new("."));

        assert_eq!(result.status, "passed");
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout_tail.contains("eval_runner_real_process"));
        assert!(!result.timed_out);
    }

    #[test]
    fn eval_suite_validation_requires_evidence_and_success_criteria() {
        let mut suite = builtin_eval_suite("false_completion_guard").unwrap();
        suite.cases[0].verifier.evidence.clear();
        suite.cases[0].verifier.success_criteria.clear();
        let errors = validate_eval_suite(&suite).unwrap_err();
        assert!(errors.iter().any(|error| error.contains("evidence")));
        assert!(errors
            .iter()
            .any(|error| error.contains("success criteria")));
    }

    #[test]
    fn orchestration_runs_agent_verifiers_and_persists_report() {
        let db = temp_db();
        let suite = builtin_eval_suite("false_completion_guard").unwrap();
        let options = EvalRunOptions {
            suite_id: suite.id.clone(),
            case_ids: vec![suite.cases[0].id.clone()],
            cwd: None,
            claimed_complete: false,
        };
        let report = run_eval_orchestration_with_executors(
            &db,
            &suite,
            &options,
            Path::new("."),
            &PassingAgent,
            &FakeExecutor {
                statuses: BTreeMap::new(),
            },
            Some("openai".to_string()),
            Some("gpt-4o-mini".to_string()),
        )
        .unwrap();
        assert_eq!(
            report.case_results[0].agent_result.as_ref().unwrap().status,
            "passed"
        );
        let stored = db.get_eval_run_record(&report.id).unwrap().unwrap();
        assert_eq!(stored.provider.as_deref(), Some("openai"));
        assert_eq!(stored.report["id"], report.id);
    }
}
