use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::storage::{
    CreateWorkspaceLifecyclePayload, LocalDb, RecordWorkspaceSetupEventPayload, StorageError,
    StorageResult, WorkspaceLifecycleRecord, WorkspaceLifecycleSnapshot,
};
use crate::tools::execution_isolation::is_sensitive_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLifecycleSpec {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    pub root_path: String,
    #[serde(default)]
    pub sandbox_backend: Option<String>,
    #[serde(default)]
    pub setup_script: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCwdVerdict {
    pub allowed: bool,
    pub workspace_root: String,
    pub cwd: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSetupRunOptions {
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitHookSpec {
    pub hook_name: String,
    pub command: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitHookInstallReport {
    pub workspace_id: String,
    pub hook_name: String,
    pub hook_path: String,
    pub installed: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceLifecycleRuntime {
    db: LocalDb,
}

impl WorkspaceLifecycleRuntime {
    pub fn new(db: LocalDb) -> Self {
        Self { db }
    }

    pub fn create(&self, spec: WorkspaceLifecycleSpec) -> StorageResult<WorkspaceLifecycleRecord> {
        let backend = spec.sandbox_backend.as_deref().unwrap_or("local");
        let fallback_reason = (backend == "local").then_some(
            "OS sandbox backend not active; command workspace boundary and env allowlist are enforced"
                .to_string(),
        );
        let record = self
            .db
            .create_workspace_lifecycle(CreateWorkspaceLifecyclePayload {
                id: spec.id,
                session_id: spec.session_id,
                run_id: spec.run_id,
                root_path: spec.root_path,
                sandbox_backend: Some(backend.to_string()),
                fallback_reason,
                setup_script: spec.setup_script,
                audit: json!({
                    "source": "workspace_lifecycle_runtime",
                    "policy": "workspace_root_hard_boundary"
                }),
            })?;
        self.db
            .record_workspace_setup_event(RecordWorkspaceSetupEventPayload {
                workspace_id: record.id.clone(),
                stage: "created".to_string(),
                status: "succeeded".to_string(),
                command: None,
                exit_code: None,
                output_tail: None,
                reason: "workspace lifecycle record created".to_string(),
            })?;
        Ok(record)
    }

    pub fn record_setup_result(
        &self,
        workspace_id: &str,
        command: Option<String>,
        exit_code: Option<i64>,
        output_tail: Option<String>,
    ) -> StorageResult<WorkspaceLifecycleSnapshot> {
        let status = if exit_code.unwrap_or(0) == 0 {
            "succeeded"
        } else {
            "failed"
        };
        self.db
            .record_workspace_setup_event(RecordWorkspaceSetupEventPayload {
                workspace_id: workspace_id.to_string(),
                stage: "setup".to_string(),
                status: status.to_string(),
                command,
                exit_code,
                output_tail,
                reason: if status == "succeeded" {
                    "setup script succeeded".to_string()
                } else {
                    "setup script failed".to_string()
                },
            })?;
        self.db.get_workspace_lifecycle_snapshot(workspace_id)
    }

    pub fn run_setup_script(
        &self,
        workspace_id: &str,
        options: WorkspaceSetupRunOptions,
    ) -> StorageResult<WorkspaceLifecycleSnapshot> {
        let snapshot = self.db.get_workspace_lifecycle_snapshot(workspace_id)?;
        let Some(script) = snapshot
            .workspace
            .setup_script
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            self.db
                .record_workspace_setup_event(RecordWorkspaceSetupEventPayload {
                    workspace_id: workspace_id.to_string(),
                    stage: "setup".to_string(),
                    status: "skipped".to_string(),
                    command: None,
                    exit_code: None,
                    output_tail: None,
                    reason: "workspace has no setup script".to_string(),
                })?;
            return self.db.get_workspace_lifecycle_snapshot(workspace_id);
        };
        let root_verdict = validate_workspace_cwd(&snapshot.workspace, ".")?;
        if !root_verdict.allowed {
            self.db
                .record_workspace_setup_event(RecordWorkspaceSetupEventPayload {
                    workspace_id: workspace_id.to_string(),
                    stage: "setup".to_string(),
                    status: "failed".to_string(),
                    command: Some(script.to_string()),
                    exit_code: None,
                    output_tail: None,
                    reason: root_verdict.reason,
                })?;
            return self.db.get_workspace_lifecycle_snapshot(workspace_id);
        }
        let result = run_shell_command(
            script,
            Path::new(&root_verdict.workspace_root),
            options.timeout_ms.unwrap_or(120_000).clamp(1_000, 600_000),
        )?;
        let status = if result.exit_code == Some(0) {
            "succeeded"
        } else {
            "failed"
        };
        self.db
            .record_workspace_setup_event(RecordWorkspaceSetupEventPayload {
                workspace_id: workspace_id.to_string(),
                stage: "setup".to_string(),
                status: status.to_string(),
                command: Some(script.to_string()),
                exit_code: result.exit_code,
                output_tail: Some(result.output_tail),
                reason: if status == "succeeded" {
                    "setup script succeeded".to_string()
                } else {
                    "setup script failed or timed out".to_string()
                },
            })?;
        self.db.get_workspace_lifecycle_snapshot(workspace_id)
    }

    pub fn validate_command_binding(
        &self,
        workspace_id: &str,
        cwd: &str,
    ) -> StorageResult<WorkspaceCwdVerdict> {
        let snapshot = self.db.get_workspace_lifecycle_snapshot(workspace_id)?;
        validate_workspace_cwd(&snapshot.workspace, cwd)
    }

    pub fn install_git_hook(
        &self,
        workspace_id: &str,
        spec: WorkspaceGitHookSpec,
    ) -> StorageResult<WorkspaceGitHookInstallReport> {
        let snapshot = self.db.get_workspace_lifecycle_snapshot(workspace_id)?;
        let hook_name = normalize_hook_name(&spec.hook_name)?;
        let command = spec.command.trim();
        if command.is_empty() {
            return Err(StorageError::Validation(
                "git hook command is empty".to_string(),
            ));
        }
        let root = PathBuf::from(&snapshot.workspace.root_path)
            .canonicalize()
            .map_err(StorageError::Io)?;
        let hooks_dir = root.join(".git").join("hooks");
        if !hooks_dir.is_dir() {
            return Err(StorageError::Validation(
                "workspace does not contain .git/hooks".to_string(),
            ));
        }
        let hook_path = hooks_dir.join(&hook_name);
        if hook_path.exists() && !spec.overwrite && !is_atlas_managed_hook(&hook_path) {
            return Err(StorageError::Validation(format!(
                "git hook {hook_name} exists and is not Atlas-managed"
            )));
        }
        let body = format!(
            "#!/bin/sh\n# Atlas-managed hook. workspace_id={workspace_id}\nset -e\n{command}\n"
        );
        std::fs::write(&hook_path, body).map_err(StorageError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&hook_path)
                .map_err(StorageError::Io)?
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&hook_path, permissions).map_err(StorageError::Io)?;
        }
        self.db
            .record_workspace_setup_event(RecordWorkspaceSetupEventPayload {
                workspace_id: workspace_id.to_string(),
                stage: "git_hooks".to_string(),
                status: "succeeded".to_string(),
                command: Some(command.to_string()),
                exit_code: Some(0),
                output_tail: Some(hook_path.to_string_lossy().to_string()),
                reason: format!("installed Atlas-managed git hook {hook_name}"),
            })?;
        Ok(WorkspaceGitHookInstallReport {
            workspace_id: workspace_id.to_string(),
            hook_name,
            hook_path: hook_path.to_string_lossy().to_string(),
            installed: true,
            reason: "Atlas-managed git hook installed".to_string(),
        })
    }
}

#[derive(Debug, Clone)]
struct ShellCommandResult {
    exit_code: Option<i64>,
    output_tail: String,
}

pub fn validate_workspace_cwd(
    workspace: &WorkspaceLifecycleRecord,
    cwd: &str,
) -> StorageResult<WorkspaceCwdVerdict> {
    let root = PathBuf::from(&workspace.root_path)
        .canonicalize()
        .map_err(StorageError::Io)?;
    let raw = PathBuf::from(cwd);
    let cwd = if raw.is_absolute() {
        raw
    } else {
        root.join(raw)
    }
    .canonicalize()
    .map_err(StorageError::Io)?;
    if is_sensitive_path(&cwd) {
        return Ok(verdict(false, &root, &cwd, "cwd is sensitive"));
    }
    if !path_starts_with(&cwd, &root) {
        return Ok(verdict(false, &root, &cwd, "cwd is outside workspace root"));
    }
    Ok(verdict(true, &root, &cwd, "cwd is inside workspace root"))
}

fn verdict(allowed: bool, root: &Path, cwd: &Path, reason: &str) -> WorkspaceCwdVerdict {
    WorkspaceCwdVerdict {
        allowed,
        workspace_root: root.to_string_lossy().to_string(),
        cwd: cwd.to_string_lossy().to_string(),
        reason: reason.to_string(),
    }
}

fn path_starts_with(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let path_text = path.to_string_lossy().to_ascii_lowercase();
        let root_text = root.to_string_lossy().to_ascii_lowercase();
        path_text == root_text
            || path_text.starts_with(&format!(
                "{}{}",
                root_text.trim_end_matches(['\\', '/']),
                std::path::MAIN_SEPARATOR
            ))
    }
    #[cfg(not(windows))]
    {
        path.starts_with(root)
    }
}

fn run_shell_command(
    script: &str,
    root: &Path,
    timeout_ms: u64,
) -> StorageResult<ShellCommandResult> {
    let mut command = shell_command(script);
    command
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(StorageError::Io)?;
    let started = Instant::now();
    loop {
        if child.try_wait().map_err(StorageError::Io)?.is_some() {
            let output = child.wait_with_output().map_err(StorageError::Io)?;
            return Ok(ShellCommandResult {
                exit_code: output.status.code().map(i64::from),
                output_tail: tail_output(&output.stdout, &output.stderr, 4000),
            });
        }
        if started.elapsed() >= Duration::from_millis(timeout_ms) {
            let _ = child.kill();
            let output = child.wait_with_output().map_err(StorageError::Io)?;
            return Ok(ShellCommandResult {
                exit_code: None,
                output_tail: format!(
                    "setup timed out after {timeout_ms}ms\n{}",
                    tail_output(&output.stdout, &output.stderr, 3500)
                ),
            });
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn shell_command(script: &str) -> Command {
    #[cfg(windows)]
    {
        let mut command = Command::new("cmd");
        command.args(["/C", script]);
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = Command::new("sh");
        command.args(["-lc", script]);
        command
    }
}

fn tail_output(stdout: &[u8], stderr: &[u8], max_chars: usize) -> String {
    let mut text = String::new();
    if !stdout.is_empty() {
        text.push_str(&String::from_utf8_lossy(stdout));
    }
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(stderr));
    }
    let chars = text.chars().collect::<Vec<_>>();
    let start = chars.len().saturating_sub(max_chars);
    chars[start..].iter().collect()
}

fn normalize_hook_name(value: &str) -> StorageResult<String> {
    match value.trim() {
        "applypatch-msg" | "commit-msg" | "post-commit" | "post-merge" | "pre-commit"
        | "pre-push" | "prepare-commit-msg" => Ok(value.trim().to_string()),
        _ => Err(StorageError::Validation(
            "unsupported git hook name".to_string(),
        )),
    }
}

fn is_atlas_managed_hook(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.contains("Atlas-managed hook"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_db() -> LocalDb {
        LocalDb::open(std::env::temp_dir().join(format!("atlas_workspace_{}.db", Uuid::new_v4())))
            .unwrap()
    }

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("atlas-{label}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn workspace_lifecycle_records_local_sandbox_fallback() {
        let db = temp_db();
        let root = temp_root("workspace");
        db.create_agent_run("run-a", None, "default").unwrap();
        let runtime = WorkspaceLifecycleRuntime::new(db.clone());
        let record = runtime
            .create(WorkspaceLifecycleSpec {
                id: Some("ws-a".to_string()),
                session_id: None,
                run_id: Some("run-a".to_string()),
                root_path: root.to_string_lossy().to_string(),
                sandbox_backend: Some("local".to_string()),
                setup_script: None,
            })
            .unwrap();
        assert_eq!(record.sandbox_status, "fallback_recorded");
        assert!(record
            .fallback_reason
            .unwrap()
            .contains("workspace boundary"));
        assert!(db
            .get_workspace_lifecycle_by_run("run-a")
            .unwrap()
            .is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_cwd_validator_rejects_outside_directory() {
        let root = temp_root("workspace-root");
        let outside = temp_root("workspace-outside");
        let workspace = WorkspaceLifecycleRecord {
            id: "ws".to_string(),
            session_id: None,
            run_id: None,
            root_path: root.to_string_lossy().to_string(),
            status: "ready".to_string(),
            setup_status: "skipped".to_string(),
            sandbox_backend: "local".to_string(),
            sandbox_status: "boundary_only".to_string(),
            fallback_reason: None,
            setup_script: None,
            audit: json!({}),
            created_at: 1,
            updated_at: 1,
            archived_at: None,
        };
        let verdict = validate_workspace_cwd(&workspace, &outside.to_string_lossy()).unwrap();
        assert!(!verdict.allowed);
        assert!(verdict.reason.contains("outside"));
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn setup_failure_moves_workspace_to_error() {
        let db = temp_db();
        let root = temp_root("workspace-setup");
        let runtime = WorkspaceLifecycleRuntime::new(db);
        runtime
            .create(WorkspaceLifecycleSpec {
                id: Some("ws-setup".to_string()),
                session_id: None,
                run_id: None,
                root_path: root.to_string_lossy().to_string(),
                sandbox_backend: Some("local".to_string()),
                setup_script: Some("npm install".to_string()),
            })
            .unwrap();
        let snapshot = runtime
            .record_setup_result(
                "ws-setup",
                Some("npm install".to_string()),
                Some(1),
                Some("failed".to_string()),
            )
            .unwrap();
        assert_eq!(snapshot.workspace.status, "error");
        assert_eq!(snapshot.workspace.setup_status, "failed");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn setup_runner_executes_inside_workspace_and_records_output() {
        let db = temp_db();
        let root = temp_root("workspace-runner");
        let runtime = WorkspaceLifecycleRuntime::new(db);
        runtime
            .create(WorkspaceLifecycleSpec {
                id: Some("ws-runner".to_string()),
                session_id: None,
                run_id: None,
                root_path: root.to_string_lossy().to_string(),
                sandbox_backend: Some("local".to_string()),
                setup_script: Some(setup_echo_command()),
            })
            .unwrap();
        let snapshot = runtime
            .run_setup_script(
                "ws-runner",
                WorkspaceSetupRunOptions {
                    timeout_ms: Some(10_000),
                },
            )
            .unwrap();
        assert_eq!(snapshot.workspace.setup_status, "succeeded");
        assert!(snapshot
            .events
            .iter()
            .any(|event| event.stage == "setup" && event.output_tail.contains("atlas-setup-ok")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn command_binding_uses_workspace_id_boundary() {
        let db = temp_db();
        let root = temp_root("workspace-binding");
        let outside = temp_root("workspace-binding-outside");
        let runtime = WorkspaceLifecycleRuntime::new(db);
        runtime
            .create(WorkspaceLifecycleSpec {
                id: Some("ws-binding".to_string()),
                session_id: None,
                run_id: None,
                root_path: root.to_string_lossy().to_string(),
                sandbox_backend: Some("local".to_string()),
                setup_script: None,
            })
            .unwrap();
        let allowed = runtime.validate_command_binding("ws-binding", ".").unwrap();
        assert!(allowed.allowed);
        let rejected = runtime
            .validate_command_binding("ws-binding", &outside.to_string_lossy())
            .unwrap();
        assert!(!rejected.allowed);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn git_hook_installer_writes_atlas_managed_hook_without_overwriting_user_hook() {
        let db = temp_db();
        let root = temp_root("workspace-hook");
        let hooks = root.join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let runtime = WorkspaceLifecycleRuntime::new(db);
        runtime
            .create(WorkspaceLifecycleSpec {
                id: Some("ws-hook".to_string()),
                session_id: None,
                run_id: None,
                root_path: root.to_string_lossy().to_string(),
                sandbox_backend: Some("local".to_string()),
                setup_script: None,
            })
            .unwrap();
        let report = runtime
            .install_git_hook(
                "ws-hook",
                WorkspaceGitHookSpec {
                    hook_name: "pre-commit".to_string(),
                    command: "echo atlas-hook".to_string(),
                    overwrite: false,
                },
            )
            .unwrap();
        assert!(report.installed);
        assert!(std::fs::read_to_string(&report.hook_path)
            .unwrap()
            .contains("Atlas-managed hook"));
        let _ = std::fs::remove_dir_all(root);
    }

    fn setup_echo_command() -> String {
        #[cfg(windows)]
        {
            "echo atlas-setup-ok".to_string()
        }
        #[cfg(not(windows))]
        {
            "printf atlas-setup-ok".to_string()
        }
    }
}
